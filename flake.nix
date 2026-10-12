{
  description = "Vibe - TypeScript AI agent framework with optional Rust accelerator";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    # Rust toolchain overlay - provides specific Rust versions
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Crane - composable, cacheable Rust workspace builds
    crane = {
      url = "github:ipetkov/crane";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
        rust-overlay.follows = "rust-overlay";
      };
    };

    # Pre-commit hooks as Nix derivations
    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, nixpkgs-unstable, flake-utils, rust-overlay, crane, pre-commit-hooks, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        # Supported systems
        supportedSystems = [
          "aarch64-darwin"
          "x86_64-darwin"
          "x86_64-linux"
        ];

        # Only build on supported systems
        isSupported = builtins.elem system supportedSystems;

        # Import nixpkgs for this system with rust-overlay
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # Rust toolchain - use rust-overlay to get a specific version
        # We try to get rustc_1_95_0 from the overlay, fallback to pkgs.rustc
        rustToolchain = if pkgs ? rustc_1_95_0 then pkgs.rustc_1_95_0 else pkgs.rustc;

        # Crane library configured with our toolchain
        craneLib = crane.mkLib pkgs;

        # Node.js 22
        nodejs = pkgs.nodejs_22;

        # Pre-commit hooks - use nixpkgs version
        pre-commit = pkgs.pre-commit;

        # Development shell base packages
        devPackages = with pkgs; [
          rustToolchain
          nodejs
          git
          biome
          pre-commit
          jq
          curl
          coreutils
          findutils
        ];

        # Bun - we use the nixpkgs version for simplicity; consider pinning to 1.3.3 via nixpkgs-unstable or fetchzip later
        bun = pkgs.bun;

        # Minimal CI packages (no dev tools)
        ciPackages = with pkgs; [
          rustToolchain
          bun
          nodejs
          git
        ];

        # Rust-only CI packages
        ciRustPackages = with pkgs; [
          rustToolchain
          bun
          nodejs
          git
        ];

        # Environment variables for the dev shell
        devShellEnv = {
          NODE_ENV = "development";
          CI = "";
          VIBE_ROOT = "$PWD";
          CARGO_HOME = "$PWD/.cargo";
          COREPACK_ENABLE_STRICT = "0";
        };

        # CI environment variables
        ciShellEnv = devShellEnv // { CI = "true"; TERM = "dumb"; };

      in {
        devShells = {
          # Default development shell
          default = pkgs.mkShell {
            name = "vibe-dev";
            packages = devPackages;
            shellHook = ''
              # Project root
              export VIBE_ROOT="$PWD"
              
              # Cargo home in project (avoids ~/.cargo pollution)
              export CARGO_HOME="$PWD/.cargo"
              
              # Auto-install bun dependencies if missing
              if [ ! -d "node_modules" ]; then
                echo "→ Installing bun dependencies..."
                bun install --frozen-lockfile
              fi
              
              # Prompt to install pre-commit hooks if not present
              if [ ! -f ".git/hooks/pre-commit" ]; then
                echo "→ Run 'pre-commit install --hook-type pre-commit --hook-type commit-msg' to install git hooks"
              fi
              
              echo "✓ Vibe dev environment ready"
              echo "  rustc: $(rustc --version)"
              echo "  bun:   $(bun --version)"
              echo "  node:  $(node --version)"
            '';
            # Set environment variables
            VIBE_ROOT = "$PWD";
            CARGO_HOME = "$PWD/.cargo";
            NODE_ENV = "development";
            CI = "";
            COREPACK_ENABLE_STRICT = "0";
          };

          # Development shell with debugging tools
          debug = pkgs.mkShell {
            name = "vibe-debug";
            packages = devPackages ++ with pkgs; [ gdb lldb strace ltrace ];
            shellHook = ''
              export VIBE_ROOT="$PWD"
              export CARGO_HOME="$PWD/.cargo"
              echo "✓ Vibe debug environment ready"
              echo "  Available: gdb, lldb, strace, ltrace"
            '';
          };

          # Development shell for profiling
          profile = pkgs.mkShell {
            name = "vibe-profile";
            packages = devPackages ++ with pkgs; [ perf valgrind ];
            shellHook = ''
              export VIBE_ROOT="$PWD"
              export CARGO_HOME="$PWD/.cargo"
              echo "✓ Vibe profile environment ready"
              echo "  Available: perf, valgrind"
            '';
          };

          # Minimal CI shell (lint, typecheck, build, test)
          ci = pkgs.mkShell {
            name = "vibe-ci";
            packages = ciPackages;
            shellHook = ''
              export VIBE_ROOT="$PWD"
              export CARGO_HOME="$PWD/.cargo"
              export CI="true"
              export TERM="dumb"
            '';
            VIBE_ROOT = "$PWD";
            CARGO_HOME = "$PWD/.cardo";
            CI = "true";
            TERM = "dumb";
          };

          # Rust-only CI shell (fmt, clippy, test, build)
          ci-rust = pkgs.mkShell {
            name = "vibe-ci-rust";
            packages = ciRustPackages;
            shellHook = ''
              export VIBE_ROOT="$PWD"
              export CARGO_HOME="$PWD/.cargo"
              export CI="true"
              export TERM="dumb"
            '';
            VIBE_ROOT = "$PWD";
            CARGO_HOME = "$PWD/.cargo";
            CI = "true";
            TERM = "dumb";
          };
        };

        # Package derivations (for CI checks and potential distribution)
        packages = {
          # Rust workspace build via Crane
          vibe-rust = craneLib.buildPackage {
            pname = "vibe-rust";
            version = "0.1.0";
            src = self;
            cargoExtraArgs = "--release --workspace";
            # Build all crates in the workspace
            doCheck = false;
          };

          # vibe_napi native addon build
          vibe-napi = craneLib.buildPackage {
            pname = "vibe-napi";
            version = "0.1.0";
            src = self;
            cargoExtraArgs = "--release -p vibe_napi";
            doCheck = false;
            postInstall = ''
              mkdir -p $out/lib
              find $out -name "libvibe_napi.*" -exec cp {} $out/vibe_napi.node \;
            '';
          };
        };

        # Flake checks (map 1:1 to CI gates)
        checks = {
          # Rust checks - use nixpkgs toolchain with source
          rust-fmt = pkgs.runCommand "rust-fmt" {
            nativeBuildInputs = [ pkgs.rustc pkgs.cargo ];
            src = self;
          } ''
            cd $src
            cargo fmt --all --check
          '';

          rust-clippy = pkgs.runCommand "rust-clippy" {
            nativeBuildInputs = [ pkgs.rustc pkgs.cargo ];
            src = self;
          } ''
            cd $src
            cargo clippy --workspace --all-targets -- -D warnings
          '';

          rust-test = pkgs.runCommand "rust-test" {
            nativeBuildInputs = [ pkgs.rustc pkgs.cargo ];
            src = self;
          } ''
            cd $src
            cargo test --workspace
          '';

          rust-bench = pkgs.runCommand "rust-bench" {
            nativeBuildInputs = [ pkgs.rustc pkgs.cargo ];
            src = self;
          } ''
            cd $src
            cargo bench --workspace --no-run
          '';

          rust-build-napi = pkgs.runCommand "rust-build-napi" {
            nativeBuildInputs = [ pkgs.rustc pkgs.cargo ];
            src = self;
          } ''
            cd $src
            cargo build --release -p vibe_napi
          '';

          # TypeScript checks (run in CI via nix develop .#ci --command)
          # Not run as flake checks since they need full dev environment
        };

        # Formatter for `nix fmt`
        formatter = pkgs.runCommand "formatter" {
          nativeBuildInputs = [ pkgs.nixfmt-rfc-style ];
        } ''
          mkdir -p $out
          nixfmt-rfc-style flake.nix > $out/formatted.nix
          cp $out/formatted.nix flake.nix
        '';
      }
    );
}
