# What `nix flake check` runs. Each check reads the source tree and does not
# build the workspace. The Bazel build and tests run in CI, not here.
{ pkgs }:
let
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;

  # Only the files the checks read. This keeps node_modules, target/ and other
  # large trees out of the Nix store.
  src = pkgs.lib.fileset.toSource {
    root = ../.;
    fileset = pkgs.lib.fileset.unions ([
      ../Cargo.toml
      ../Cargo.lock
      ../deny.toml
      ../rust-toolchain.toml
      ../rustfmt.toml
      ../clippy.toml
      ../bazel/tools/buildkite/ci.sh
      ../flake.nix
      ../nix
    ] ++ map (dir: pkgs.lib.fileset.fileFilter (f: f.hasExt "rs" || f.hasExt "toml") dir)
      [ ../crates ../cli ../benchmarks ../sdks/rust ../third_party ]);
  };
in {
  # Rust formatting, from the pinned toolchain. Clippy runs in the Rust CI job.
  rust-fmt = pkgs.runCommand "check-rust-fmt" {
    nativeBuildInputs = [ rustToolchain ];
  } ''
    cd ${src}
    cargo fmt --all --check
    touch $out
  '';

  # Nix sources, in the RFC 166 style.
  nix-fmt = pkgs.runCommand "check-nix-fmt" {
    nativeBuildInputs = [ pkgs.nixfmt-rfc-style ];
  } ''
    nixfmt --check ${src}/flake.nix
    find ${src}/nix -name '*.nix' -exec nixfmt --check {} +
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
}
