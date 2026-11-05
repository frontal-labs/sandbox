#!/bin/sh
# Builds CPython for WASI if its artifacts are missing, then runs the CPython tests on the
# sandbox's WASI host. Those tests are ignored by default, because they need the artifacts.
#
#   tools/wasm/python/conformance.sh     (needs Docker, for the build)
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT="$ROOT/tools/wasm/python/out"

if [ ! -f "$OUT/python.wasm" ] || [ ! -f "$OUT/python313.zip" ] || [ ! -f "$OUT/python-init.wasm" ]; then
  "$ROOT/tools/wasm/python/build.sh"
fi

# The artifacts must match the pins, so a changed build is noticed before anything runs.
pin() { grep "^$1 = " "$ROOT/tools/wasm/python/pins.toml" | cut -d'"' -f2; }
[ "$(b3sum --no-names "$OUT/python.wasm")" = "$(pin python_wasm)" ] || { echo "python.wasm does not match tools/wasm/python/pins.toml" >&2; exit 1; }
[ "$(b3sum --no-names "$OUT/python313.zip")" = "$(pin python313_zip)" ] || { echo "python313.zip does not match tools/wasm/python/pins.toml" >&2; exit 1; }

export SANDBOX_PYTHON_WASM="$OUT/python.wasm"
export SANDBOX_PYTHON_REACTOR="$OUT/python-init.wasm"
export SANDBOX_PYTHON_ZIP="$OUT/python313.zip"
cd "$ROOT"
cargo test -p sandbox-wasm --test python_conformance -- --ignored
cargo test -p sandbox-shell -- --ignored python
# The same cases through the pre-initialised reactor, which runs `python -c` (see reactor/).
SANDBOX_PYTHON_WASM="$OUT/python-init.wasm" cargo test -p sandbox-wasm --test python_conformance -- --ignored
