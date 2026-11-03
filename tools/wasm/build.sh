#!/bin/sh
# Builds the WASI tools and installs them as the sandbox's test fixtures, printing the
# BLAKE3 hash of each module. The hashes go into crates/sandbox-wasm/fixtures/tools/tools.toml,
# and `cargo test -p sandbox-wasm --test registry_manifest` checks them.
#
#   tools/wasm/build.sh     needs the wasm32-wasip1 target and b3sum
set -eu

ROOT=$(cd "$(dirname "$0")" && pwd)
FIXTURES="$ROOT/../../crates/sandbox-wasm/fixtures"

cd "$ROOT"
# One package per build: building them together unifies their features, which changes
# the bytes of each module and so its pinned hash.
for tool in jq rg git; do
  cargo build --release --target wasm32-wasip1 -p "${tool}_tool"
  src="$ROOT/target/wasm32-wasip1/release/${tool}_tool.wasm"
  dst="$FIXTURES/${tool}_tool.wasm"
  cp "$src" "$dst"
  printf '%s_tool.wasm %s\n' "$tool" "$(b3sum --no-names "$dst")"
done
