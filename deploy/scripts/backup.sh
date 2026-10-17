#!/usr/bin/env bash
set -euo pipefail

# backup.sh — back up sandbox state.
#
# Usage: ./backup.sh [DEST]
#   DEST: local directory or s3://bucket/prefix. Defaults to ./sandbox-backup-<date>

DEST="${1:-./sandbox-backup-$(date +%Y%m%d%H%M%S)}"
STATE_DIR="/var/lib/sandbox"

[[ -d "$STATE_DIR" ]] || { echo "state dir not found: $STATE_DIR" >&2; exit 1; }

case "$DEST" in
  s3://*)
    TMPDIR="$(mktemp -d)"
    tar -czf "$TMPDIR/state.tar.gz" -C "$(dirname "$STATE_DIR")" "$(basename "$STATE_DIR")"
    aws s3 cp "$TMPDIR/state.tar.gz" "$DEST/state-$(date +%Y%m%d%H%M%S).tar.gz"
    rm -rf "$TMPDIR"
    ;;
  *)
    mkdir -p "$DEST"
    tar -czf "$DEST/state-$(date +%Y%m%d%H%M%S).tar.gz" -C "$(dirname "$STATE_DIR")" "$(basename "$STATE_DIR")"
    ;;
esac

echo "backup complete: $DEST"
