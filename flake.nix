{
  description = "Frontal Sandbox - isolate-first execution environments for AI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ rust bun git jq curl coreutils findutils ];
          shellHook = ''
            export SANDBOX_ROOT="$PWD"
            echo "Frontal Sandbox development environment ready"
          '';
        };
      });
}
