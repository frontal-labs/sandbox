#!/usr/bin/env bash
set -euo pipefail

# restore.sh — restore sandbox state from a backup.
#
# Usage: sudo ./restore.sh <backup.tar.gz|s3://bucket/key>

BACKUP="${1:-}"
[[ -n "$BACKUP" ]] || { echo "usage: restore.sh <file|s3://...>" >&2; exit 2; }

STATE_DIR="/var/lib/sandbox"
TMPDIR="$(mktemp -d)"

case "$BACKUP" in
  s3://*)
    aws s3 cp "$BACKUP" "$TMPDIR/backup.tar.gz"
    ;;
  *)
    [[ -f "$BACKUP" ]] || { echo "backup not found: $BACKUP" >&2; exit 1; }
    cp "$BACKUP" "$TMPDIR/backup.tar.gz"
    ;;
esac

systemctl stop sandbox-server || true
rm -rf "$STATE_DIR"
mkdir -p "$STATE_DIR"
tar -xzf "$TMPDIR/backup.tar.gz" -C "$(dirname "$STATE_DIR")"
chown -R sandbox:sandbox "$STATE_DIR"
chmod 0700 "$STATE_DIR"
rm -rf "$TMPDIR"

systemctl start sandbox-server
echo "restore complete"
