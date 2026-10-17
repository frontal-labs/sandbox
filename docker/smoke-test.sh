#!/usr/bin/env bash
# =============================================================================
# Smoke test for docker/compose.yaml
# =============================================================================
# Starts an isolated stack and checks, in order:
#   1. the container becomes healthy
#   2. /v1/openapi.json answers without a key
#   3. /v1 routes reject a request without a key (401)
#   4. the API key from the first start creates a session (201)
#   5. the key and the session still work after a restart (durable state)
#
# It uses its own compose project, state volume, image tag, and host port, so
# it never touches the default stack's volume. The EXIT trap runs `down -v`,
# which removes only the smoke volume.
#
# Usage (from anywhere):  docker/smoke-test.sh
# Requires: docker with Compose v2, curl. Override the port with SMOKE_PORT.
# =============================================================================
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${SMOKE_PORT:-18080}"
BASE="http://127.0.0.1:${PORT}"

export SANDBOX_PORT="${PORT}"
export SANDBOX_STATE_VOLUME="frontal-sandbox-smoke-state"
export SANDBOX_IMAGE="frontal-sandbox:smoke"

COMPOSE=(docker compose -f "${HERE}/compose.yaml" -p frontal-sandbox-smoke)

# A nil tenant and an empty policy: the smallest valid SessionConfig.
SESSION_BODY='{
  "tenant_id": "00000000000000000000000000",
  "tier_preference": "auto",
  "limits": {},
  "network": { "default_deny": true, "rules": [] },
  "env": {},
  "cwd": "/work",
  "labels": {}
}'

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  "${COMPOSE[@]}" logs --tail=80 sandbox >&2 || true
  exit 1
}
pass() { echo "ok: $*"; }

status_of() { curl -s -o /dev/null -w '%{http_code}' "$@"; }

wait_healthy() {
  "${COMPOSE[@]}" up -d --wait --wait-timeout 180 sandbox >/dev/null \
    || fail "stack did not become healthy"
}

"${COMPOSE[@]}" up -d --build --wait --wait-timeout 300 sandbox >/dev/null \
  || fail "stack did not come up"
pass "container healthy"

code=$(status_of "${BASE}/v1/openapi.json")
[[ "${code}" == "200" ]] || fail "GET /v1/openapi.json returned ${code}, want 200"
pass "openapi.json served without a key"

code=$(status_of -X POST -H 'Content-Type: application/json' -d "${SESSION_BODY}" "${BASE}/v1/sessions")
[[ "${code}" == "401" ]] || fail "POST /v1/sessions without a key returned ${code}, want 401"
pass "session route rejects a request without a key"

# The key is printed once, on the first durable start. It is in the log because
# this is a fresh smoke volume.
KEY=$("${COMPOSE[@]}" logs sandbox 2>&1 \
  | sed -n 's/.*API key (shown once): //p' | tail -n1 | tr -d '[:space:]')
[[ -n "${KEY}" ]] || fail "no API key found in the container log"
pass "API key read from the first-start log"

RESP=$(mktemp)
trap 'rm -f "${RESP}"; cleanup' EXIT
code=$(curl -s -o "${RESP}" -w '%{http_code}' \
  -H "Authorization: Bearer ${KEY}" -H 'Content-Type: application/json' \
  -d "${SESSION_BODY}" "${BASE}/v1/sessions")
[[ "${code}" == "201" ]] || fail "POST /v1/sessions with the key returned ${code}, want 201: $(cat "${RESP}")"
SESSION_ID=$(sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p' "${RESP}")
[[ -n "${SESSION_ID}" ]] || fail "no session_id in response: $(cat "${RESP}")"
pass "session created: ${SESSION_ID}"

code=$(status_of -H "Authorization: Bearer ${KEY}" "${BASE}/v1/sessions/${SESSION_ID}")
[[ "${code}" == "200" ]] || fail "GET session returned ${code}, want 200"
pass "session readable with the key"

"${COMPOSE[@]}" restart sandbox >/dev/null
wait_healthy
pass "container healthy after restart"

code=$(status_of -H "Authorization: Bearer ${KEY}" "${BASE}/v1/sessions/${SESSION_ID}")
# Known server gap: create_session (crates/sandbox-server/src/api.rs) does not
# call persist(), so a session that was only created is lost on restart. Snapshot,
# sleep, wake, and destroy do persist. A 401 here would mean the key was lost,
# which is a volume problem, so the two cases are reported differently.
[[ "${code}" != "401" ]] || fail "after restart the key was rejected (401): the state volume did not keep the key"
[[ "${code}" == "200" ]] || fail "after restart GET session returned ${code}, want 200. Known gap: create_session does not persist (api.rs)"
pass "key and session survived the restart"

echo "All smoke checks passed."
