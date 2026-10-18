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

# Refuse archives with absolute paths, parent references, or links before
# extracting anything. Only the sandbox/ tree is allowed.
if tar -tvzf "$TMPDIR/backup.tar.gz" | awk '
    $1 ~ /^l/ || $1 ~ /^h/ { bad = 1 }
    $NF ~ /^\// || $NF ~ /(^|\/)\.\.(\/|$)/ { bad = 1 }
    $NF !~ /^sandbox(\/|$)/ { bad = 1 }
    $NF ~ /^sandbox\/?$/ { found = 1 }
    END { exit (bad || !found) }'; then
  :
else
  echo "backup contains unsafe entries; refusing to restore" >&2
  rm -rf "$TMPDIR"
  exit 1
fi

# Extract into a scratch directory first, then swap it in, so a bad archive
# cannot leave a half-restored state dir behind.
STAGE="$(mktemp -d)"
tar -xzf "$TMPDIR/backup.tar.gz" -C "$STAGE" --no-same-owner --no-same-permissions
if [[ ! -d "$STAGE/sandbox" || -L "$STAGE/sandbox" ]]; then
  echo "backup does not contain a sandbox/ directory; refusing to restore" >&2
  rm -rf "$STAGE" "$TMPDIR"
  exit 1
fi

# Move the live state aside instead of deleting it, and only remove the old
# copy once the new one is in place.
systemctl stop sandbox-server || true
OLD="$STATE_DIR.old.$$"
[[ -e "$STATE_DIR" ]] && mv "$STATE_DIR" "$OLD"
mkdir -p "$(dirname "$STATE_DIR")"
if ! mv "$STAGE/sandbox" "$STATE_DIR"; then
  [[ -e "$OLD" ]] && mv "$OLD" "$STATE_DIR"
  echo "restore failed; previous state put back" >&2
  exit 1
fi
rm -rf "$STAGE" "$OLD"
chown -R sandbox:sandbox "$STATE_DIR"
chmod 0700 "$STATE_DIR"
rm -rf "$TMPDIR"

systemctl start sandbox-server
echo "restore complete"
