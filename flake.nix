{
  description = "Frontal Sandbox development environment and repository checks";

  # Nix owns the developer environment and the repository checks. Bazel owns the
  # build (see MODULE.bazel and docs/build-system.md). Bazel is fetched by
  # bazelisk from .bazelversion, so it is not pinned here.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
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

        # The toolchain comes from rust-toolchain.toml, the same file rustup reads,
        # so Nix, rustup and CI cannot drift apart.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Formatters and linters the repository uses, plus what the Bazel build
        # needs on the host (bazelisk, protoc for the Cargo build script).
        tools = with pkgs; [
          bazelisk
          protobuf
          nodejs_22
          python312
          ruff
          biome
          taplo
          shfmt
          shellcheck
          nixfmt-rfc-style
          cargo-deny
          cargo-audit
          jq
          curl
          git
        ];

        # Repository checks, run by `nix flake check`. Each one reads the source
        # tree only and does not build the workspace. The Bazel build is checked
        # separately in CI. The source is filtered so node_modules and target/
        # stay out of the Nix store.
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions ([
            ./Cargo.toml
            ./Cargo.lock
            ./deny.toml
            ./rust-toolchain.toml
            ./rustfmt.toml
            ./clippy.toml
            ./bazel/tools/buildkite/ci.sh
            ./flake.nix
          ] ++ map (dir: pkgs.lib.fileset.fileFilter (f: f.hasExt "rs" || f.hasExt "toml") dir)
            [ ./crates ./cli ./benchmarks ./sdks/rust ./third_party ]);
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain ] ++ tools;
          shellHook = ''
            export SANDBOX_ROOT="$PWD"
            echo "Frontal Sandbox dev shell: $(rustc --version), $(bazelisk version 2>/dev/null | grep -m1 Bazel || echo 'bazelisk (Bazel pinned by .bazelversion)')"
          '';
        };

        checks = {
          # Rust: rustfmt, from the pinned toolchain. Clippy runs in the Rust CI job.
          rust-fmt = pkgs.runCommand "check-rust-fmt" {
            nativeBuildInputs = [ rustToolchain ];
          } ''
            cd ${src}
            cargo fmt --all --check
            touch $out
          '';

          # Nix sources, formatted with the RFC 166 style.
          nix-fmt = pkgs.runCommand "check-nix-fmt" {
            nativeBuildInputs = [ pkgs.nixfmt-rfc-style ];
          } ''
            nixfmt --check ${src}/flake.nix
            touch $out
          '';

          # Shell scripts in the repository.
          shell-lint = pkgs.runCommand "check-shell" {
            nativeBuildInputs = [ pkgs.shellcheck pkgs.shfmt ];
          } ''
            cd ${src}
            shellcheck bazel/tools/buildkite/ci.sh
            shfmt -d bazel/tools/buildkite/ci.sh
            touch $out
          '';

          # TOML configuration: Cargo, rust-toolchain, deny and clippy config.
          toml-fmt = pkgs.runCommand "check-toml" {
            nativeBuildInputs = [ pkgs.taplo ];
          } ''
            cd ${src}
            taplo fmt --check Cargo.toml deny.toml rust-toolchain.toml rustfmt.toml clippy.toml
            touch $out
          '';
        };

        formatter = pkgs.nixfmt-rfc-style;
      });
}
