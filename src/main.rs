use std::net::SocketAddr;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "opencode-free-gateway",
    about = "OpenAI-compatible HTTP gateway bridging OpenCode free models to OMP"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1", env = "GATEWAY_HOST")]
    host: String,

    #[arg(short, long, default_value_t = 4006, env = "GATEWAY_PORT")]
    port: u16,

    #[arg(
        long,
        default_value = "/run/current-system/sw/bin/opencode",
        env = "OPENCODE_BIN"
    )]
    opencode_bin: String,
}

#[derive(Clone)]
struct AppState {
    args: Args,
}

#[derive(Deserialize, Debug)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    #[allow(dead_code)]
    temperature: Option<f32>,
    #[serde(default)]
    #[allow(dead_code)]
    max_tokens: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct ChatMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ModelItem {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: &'static str,
}

#[derive(Serialize)]
struct ModelListResponse {
    object: &'static str,
    data: Vec<ModelItem>,
}

#[derive(Serialize)]
struct ChatCompletionResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
}

#[derive(Serialize)]
struct ChatChoice {
    index: usize,
    message: ChatResponseMessage,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChatResponseMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Serialize)]
struct ChatChunk {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChunkChoice>,
}

#[derive(Serialize)]
struct ChatChunkChoice {
    index: usize,
    delta: ChatChunkDelta,
    finish_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct ChatChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

const FREE_MODELS: &[&str] = &[
    "deepseek-v4.1-flash-free",
    "big-pickle",
    "mimo-v2.6-flash-free",
    "nemotron-3.5-lightning-free",
    "nemotron-3.5-super-vl-free",
    "space-bunny-free",
    "glm-5.3-flash-free",
    "beam-free",
    "longcat-2.5-preview-free",
];

fn normalize_model(id: &str) -> String {
    let clean = id.trim();
    if clean.starts_with("opencode/") {
        clean.to_string()
    } else {
        format!("opencode/{}", clean)
    }
}

fn extract_text_from_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let mut res = String::new();
            for item in arr {
                if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                    res.push_str(t);
                    res.push('\n');
                }
            }
            res
        }
        _ => val.to_string(),
    }
}

fn build_prompt(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    let mut system_parts = Vec::new();

    for msg in messages {
        let text = extract_text_from_value(&msg.content);
        if msg.role == "system" {
            system_parts.push(text);
        } else {
            let role_name = match msg.role.as_str() {
                "user" => "User",
                "assistant" => "Assistant",
                "tool" => "Tool Result",
                _ => "Message",
            };
            parts.push(format!("[{}]: {}", role_name, text));
        }
    }

    let mut full = String::new();
    if !system_parts.is_empty() {
        full.push_str("# Instructions\n");
        full.push_str(&system_parts.join("\n\n"));
        full.push_str("\n\n---\n\n");
    }

    if !parts.is_empty() {
        full.push_str(&parts.join("\n\n---\n\n"));
    } else {
        full.push_str("Hello");
    }

    full
}

async fn list_models() -> Json<ModelListResponse> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut data = Vec::new();
    for &m in FREE_MODELS {
        data.push(ModelItem {
            id: format!("opencode/{}", m),
            object: "model",
            created: now,
            owned_by: "opencode",
        });
        data.push(ModelItem {
            id: m.to_string(),
            object: "model",
            created: now,
            owned_by: "opencode",
        });
    }

    Json(ModelListResponse {
        object: "list",
        data,
    })
}

async fn handle_chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Response, (StatusCode, String)> {
    let model_id = normalize_model(&req.model);
    let prompt = build_prompt(&req.messages);

    let mut cmd = Command::new(&state.args.opencode_bin);
    cmd.kill_on_drop(true);
    cmd.current_dir("/tmp");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd.arg("run")
        .arg("--pure")
        .arg("--dir")
        .arg("/tmp")
        .arg("--format")
        .arg("json")
        .arg("-m")
        .arg(&model_id)
        .arg(&prompt);

    let mut child = cmd.spawn().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to spawn opencode: {}", e),
        )
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to capture child stdout".to_string(),
        )
    })?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let completion_id = format!("chatcmpl-{}", now);

    if req.stream {
        let stream = async_stream::stream! {
            let mut reader = BufReader::new(stdout).lines();
            let mut emitted_first = false;

            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                    let ev_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if ev_type == "text" {
                        if let Some(text) = val.get("part").and_then(|p| p.get("text")).and_then(|t| t.as_str()) {
                            if !emitted_first {
                                let chunk = ChatChunk {
                                    id: completion_id.clone(),
                                    object: "chat.completion.chunk",
                                    created: now,
                                    model: model_id.clone(),
                                    choices: vec![ChatChunkChoice {
                                        index: 0,
                                        delta: ChatChunkDelta {
                                            role: Some("assistant"),
                                            content: Some(text.to_string()),
                                        },
                                        finish_reason: None,
                                    }],
                                };
                                emitted_first = true;
                                if let Ok(json) = serde_json::to_string(&chunk) {
                                    yield Ok::<_, std::io::Error>(format!("data: {}\n\n", json));
                                }
                            } else {
                                let chunk = ChatChunk {
                                    id: completion_id.clone(),
                                    object: "chat.completion.chunk",
                                    created: now,
                                    model: model_id.clone(),
                                    choices: vec![ChatChunkChoice {
                                        index: 0,
                                        delta: ChatChunkDelta {
                                            role: None,
                                            content: Some(text.to_string()),
                                        },
                                        finish_reason: None,
                                    }],
                                };
                                if let Ok(json) = serde_json::to_string(&chunk) {
                                    yield Ok::<_, std::io::Error>(format!("data: {}\n\n", json));
                                }
                            }
                        }
                    } else if ev_type == "step_finish" {
                        let chunk = ChatChunk {
                            id: completion_id.clone(),
                            object: "chat.completion.chunk",
                            created: now,
                            model: model_id.clone(),
                            choices: vec![ChatChunkChoice {
                                index: 0,
                                delta: ChatChunkDelta {
                                    role: None,
                                    content: None,
                                },
                                finish_reason: Some("stop"),
                            }],
                        };
                        if let Ok(json) = serde_json::to_string(&chunk) {
                            yield Ok::<_, std::io::Error>(format!("data: {}\n\n", json));
                        }
                        yield Ok::<_, std::io::Error>("data: [DONE]\n\n".to_string());
                        break;
                    }
                }
            }
            let _ = child.wait().await;
        };

        let body = Body::from_stream(stream);
        let mut response = Response::new(body);
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        response
            .headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        Ok(response)
    } else {
        let mut reader = BufReader::new(stdout).lines();
        let mut full_text = String::new();

        while let Ok(Some(line)) = reader.next_line().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                let ev_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if ev_type == "text" {
                    if let Some(text) = val
                        .get("part")
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        full_text.push_str(text);
                    }
                } else if ev_type == "step_finish" {
                    break;
                }
            }
        }
        let _ = child.wait().await;

        let resp = ChatCompletionResponse {
            id: completion_id,
            object: "chat.completion",
            created: now,
            model: model_id,
            choices: vec![ChatChoice {
                index: 0,
                message: ChatResponseMessage {
                    role: "assistant",
                    content: full_text,
                },
                finish_reason: "stop",
            }],
            usage: ChatUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
        };

        Ok(Json(resp).into_response())
    }
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opencode_free_gateway=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    let state = AppState { args: args.clone() };

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .route("/v1/chat/completions", post(handle_chat))
        .route("/chat/completions", post(handle_chat))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::new(args.host.parse()?, args.port);
    info!(target: "gateway", "OpenCode Free Gateway listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            info!(target: "gateway", "Shutdown signal received.");
        })
        .await?;

    Ok(())
}
