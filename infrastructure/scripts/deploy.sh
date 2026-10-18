#!/usr/bin/env bash
set -euo pipefail

# deploy.sh — install the sandbox binary, systemd units, and config on a host.
#
# Usage: sudo ./deploy.sh --binary <path> [--config <path>] [--no-systemd]
#
# Installs:
#   /usr/local/bin/frontal-sandbox          static binary
#   /etc/sandbox/config.toml        config (from --config or defaults.toml)
#   /etc/systemd/system/*.service   systemd units (unless --no-systemd)
#
# Then starts sandbox-server (and sandbox-scheduler on a standalone host).

BINARY=""
CONFIG=""
NO_SYSTEMD=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --no-systemd) NO_SYSTEMD=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$BINARY" ]]; then
  echo "usage: deploy.sh --binary <path> [--config <path>]" >&2
  exit 2
fi

if [[ ! -x "$BINARY" ]]; then
  echo "binary not found or not executable: $BINARY" >&2
  exit 1
fi

INFRA_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 1. Binary.
install -m 0755 "$BINARY" /usr/local/bin/frontal-sandbox

# 2. Config.
CONFIG_DIR="/etc/sandbox"
install -d -m 0755 "$CONFIG_DIR"

if [[ -n "$CONFIG" ]]; then
  install -m 0600 "$CONFIG" "$CONFIG_DIR/config.toml"
elif [[ -f "$INFRA_DIR/config/defaults.toml" ]]; then
  install -m 0600 "$INFRA_DIR/config/defaults.toml" "$CONFIG_DIR/config.toml"
fi

# 3. Systemd units.
if [[ "$NO_SYSTEMD" -eq 0 ]] && command -v systemctl >/dev/null 2>&1; then
  UNIT_DIR="/etc/systemd/system"
  install -m 0644 "$INFRA_DIR/systemd/sandbox-server.service" "$UNIT_DIR/"
  install -m 0644 "$INFRA_DIR/systemd/sandbox-scheduler.service" "$UNIT_DIR/"
  install -m 0644 "$INFRA_DIR/systemd/sandbox-worker@.service"   "$UNIT_DIR/"
  install -m 0644 "$INFRA_DIR/systemd/sandbox.target"            "$UNIT_DIR/"
  systemctl daemon-reload
  systemctl enable --now sandbox-server.service
  systemctl enable --now sandbox-scheduler.service
fi

echo "deploy complete. Check: systemctl status sandbox-server"
