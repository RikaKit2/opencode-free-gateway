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
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "opencode-free-gateway";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          postPatch = ''
            rm -rf .cargo/config.toml
          '';

          meta = with pkgs.lib; {
            description = "OpenCode Free Gateway: Fast OpenAI-compatible proxy for OpenCode free models";
            license = licenses.mit;
            mainProgram = "opencode-free-gateway";
            platforms = [ "x86_64-linux" ];
          };
        };
      };
    };
}
