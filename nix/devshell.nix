# The developer environment, entered with `nix develop` or through direnv.
#
# The Rust toolchain comes from rust-toolchain.toml, the same file rustup reads,
# so Nix, rustup and CI cannot drift apart. Bazel comes from the `bazel`
# package, which runs bazelisk. The build works without this shell too.
{ pkgs, bazel }:
let
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
in
pkgs.mkShell {
  packages = [
    rustToolchain
    bazel

    # Build inputs the Bazel build and the Cargo build script need on the host.
    pkgs.protobuf
    pkgs.nodejs_22
    pkgs.python312

    # Formatters and linters.
    pkgs.ruff
    pkgs.biome
    pkgs.taplo
    pkgs.shfmt
    pkgs.shellcheck
    pkgs.nixfmt-rfc-style
    pkgs.cargo-deny
    pkgs.cargo-audit

    # Utilities the scripts use.
    pkgs.jq
    pkgs.curl
    pkgs.git
  ];

  shellHook = ''
    export SANDBOX_ROOT="$PWD"
    echo "Frontal Sandbox dev shell: $(rustc --version)"
  '';
}
