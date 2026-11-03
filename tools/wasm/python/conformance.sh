#!/bin/sh
# Builds CPython for WASI if its artifacts are missing, then runs the CPython tests on the
# sandbox's WASI host. Those tests are ignored by default, because they need the artifacts.
#
#   tools/wasm/python/conformance.sh     (needs Docker, for the build)
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT="$ROOT/tools/wasm/python/out"

if [ ! -f "$OUT/python.wasm" ] || [ ! -f "$OUT/python313.zip" ]; then
  "$ROOT/tools/wasm/python/build.sh"
fi

export SANDBOX_PYTHON_WASM="$OUT/python.wasm"
export SANDBOX_PYTHON_ZIP="$OUT/python313.zip"
cd "$ROOT"
cargo test -p sandbox-wasm --test python_conformance -- --ignored
cargo test -p sandbox-shell -- --ignored python_runs_from_the_registry
