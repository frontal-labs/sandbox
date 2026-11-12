#!/bin/sh
# Builds sandbox and runs the MCP coding agent example end to end.
# (`frontal-sandbox` is a legacy alias that works identically.)
#
#   cargo build -p cli --bin sandbox && examples/mcp-agent/smoke.sh
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN="${SANDBOX_BIN:-${FRONTAL_SANDBOX_BIN:-$ROOT/target/debug/sandbox}}"

if [ ! -x "$BIN" ]; then
  cargo build -p cli --bin sandbox
fi

export FRONTAL_SANDBOX_BIN="$BIN"
cd "$ROOT/examples/mcp-agent"
npm install --ignore-scripts
npx tsx src/index.ts
