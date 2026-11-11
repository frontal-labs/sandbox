#!/bin/sh
# Builds frontal-sandbox and runs the MCP coding agent example end to end.
#
#   cargo build -p cli --bin frontal-sandbox && examples/mcp-agent/smoke.sh
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN="${FRONTAL_SANDBOX_BIN:-$ROOT/target/debug/frontal-sandbox}"

if [ ! -x "$BIN" ]; then
  cargo build -p cli --bin frontal-sandbox
fi

export FRONTAL_SANDBOX_BIN="$BIN"
cd "$ROOT/examples/mcp-agent"
npm install --ignore-scripts
npx tsx src/index.ts
