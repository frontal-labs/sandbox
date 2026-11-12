#!/bin/sh
# Drives `sandbox mcp` over stdio the way an MCP client does, and checks the replies.
# (`frontal-sandbox` is a legacy alias that works identically.)
#
#   cargo build -p cli --bin sandbox && examples/mcp/smoke.sh
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN="${SANDBOX_BIN:-${FRONTAL_SANDBOX_BIN:-$ROOT/target/debug/sandbox}}"

out=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"/a.txt","content":"x y\n"}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"exec","arguments":{"command":"wc -w < /a.txt"}}}' \
  | "$BIN" mcp)

printf '%s\n' "$out" | grep -q '"protocolVersion":"2024-11-05"' || { echo "initialize failed" >&2; exit 1; }
printf '%s\n' "$out" | grep -q '"id":3' || { echo "exec reply missing" >&2; exit 1; }
printf '%s\n' "$out" | grep -qF '2\n\n[exit 0]' || { echo "exec did not count the two words" >&2; exit 1; }
echo "MCP smoke test passed"
