#!/usr/bin/env bash
set -euo pipefail

# smoke.sh — end-to-end health check against a live sandbox endpoint.
#
# Usage: ./smoke.sh [BASE_URL] [API_KEY]
#   BASE_URL defaults to http://127.0.0.1:8080
#   API_KEY defaults to SANDBOX_API_KEY env var
#
# Checks /v1/openapi.json, creates a session, execs `echo hi`, and destroys
# the session. Exits non-zero on any failure.

BASE_URL="${1:-http://127.0.0.1:8080}"
API_KEY="${API_KEY:-${SANDBOX_API_KEY:-}}"

fail() { echo "smoke: $*" >&2; exit 1; }

echo "smoke: health check"
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE_URL/v1/openapi.json" || true)
[[ "$code" == "200" ]] || fail "openapi returned $code"

if [[ -z "$API_KEY" ]]; then
  echo "smoke: no API key; skipping authenticated paths"
  echo "smoke: done (unauthenticated only)"
  exit 0
fi

echo "smoke: create session"
body=$(curl -s -X POST "$BASE_URL/v1/sessions" \
  -H "Authorization: Bearer $API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"config":{}}' || true)
[[ -n "$body" ]] || fail "session create returned empty"
session_id=$(echo "$body" | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])' 2>/dev/null || true)
[[ -n "$session_id" ]] || fail "could not parse session id from: $body"

echo "smoke: exec echo hi"
out=$(curl -s -X POST "$BASE_URL/v1/sessions/$session_id/exec" \
  -H "Authorization: Bearer $API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"command":"echo hi"}' || true)
echo "$out" | grep -qi "hi" || fail "exec output did not contain hi: $out"

echo "smoke: destroy session"
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "$BASE_URL/v1/sessions/$session_id" \
  -H "Authorization: Bearer $API_KEY" || true)
[[ "$code" =~ ^2 ]] || fail "destroy returned $code"

echo "smoke: done"
