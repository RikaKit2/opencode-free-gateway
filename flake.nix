{
  description = "OpenCode Free Gateway: Fast OpenAI-compatible proxy for OpenCode free models";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      packages.${system} = {
        default = pkgs.stdenvNoCC.mkDerivation {
          pname = "opencode-free-gateway";
          version = "0.1.0";
          src = pkgs.fetchurl {
            url = "https://github.com/RikaKit2/opencode-free-gateway/releases/download/v0.1.0/opencode-free-gateway-v0.1.0-x86_64-unknown-linux-gnu.tar.gz";
            sha256 = "sha256-p9VjBBzP2JGOhWJFcZy3drxCm+L6qGUwhtm82NnSx/0=";
          };

          sourceRoot = ".";
          dontConfigure = true;
          dontBuild = true;

          installPhase = ''
            runHook preInstall
            install -m755 -D opencode-free-gateway $out/bin/opencode-free-gateway
            runHook postInstall
          '';

          meta = with pkgs.lib; {
            description = "OpenCode Free Gateway: Fast OpenAI-compatible proxy for OpenCode free models";
            license = licenses.mit;
            mainProgram = "opencode-free-gateway";
            platforms = [ "x86_64-linux" ];
          };
        };

        from-source = pkgs.rustPlatform.buildRustPackage {
          pname = "opencode-free-gateway";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          postPatch = ''
            rm -rf .cargo/config.toml
          '';
        };
      };
    };
}
