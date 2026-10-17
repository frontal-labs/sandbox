#!/usr/bin/env bash
set -euo pipefail

# upgrade.sh — blue-green worker rollout.
#
# Drains workers one at a time, stops the old unit, starts the new binary,
# and verifies health before moving on. For single-node deploys; the
# Kubernetes equivalent is a Deployment rollingUpdate.
#
# Usage: sudo ./upgrade.sh --binary <path> [--tenant <id>]

BINARY=""
TENANT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --tenant) TENANT="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

[[ -x "$BINARY" ]] || { echo "binary not found: $BINARY" >&2; exit 1; }

if [[ -n "$TENANT" ]]; then
  UNIT="sandbox-worker@$TENANT.service"
else
  UNIT="sandbox-server.service"
fi

echo "upgrade: stop $UNIT"
systemctl stop "$UNIT"

echo "upgrade: install binary"
install -m 0755 "$BINARY" /usr/local/bin/frontal-sandbox

echo "upgrade: start $UNIT"
systemctl start "$UNIT"

echo "upgrade: verify health"
for _ in $(seq 1 30); do
  if systemctl is-active --quiet "$UNIT"; then
    echo "upgrade: $UNIT healthy"
    exit 0
  fi
  sleep 1
done
echo "upgrade: $UNIT did not become healthy" >&2
systemctl status "$UNIT" --no-pager || true
exit 1
