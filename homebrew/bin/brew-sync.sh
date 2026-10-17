#!/usr/bin/env bash
# Brewfile sync tool: dump / check / apply modes around brew bundle.
#
# Usage:
#   homebrew/bin/brew-sync.sh dump   # regenerate Brewfile + snapshot
#   homebrew/bin/brew-sync.sh check  # diff install vs Brewfile
#   homebrew/bin/brew-sync.sh apply  # install Brewfile after confirmation
#
# Targets homebrew/Brewfile via HOMEBREW_BUNDLE_FILE.
set -euo pipefail

HOMEBREW_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BREWFILE="$HOMEBREW_DIR/Brewfile"
SNAPSHOT_DIR="$HOMEBREW_DIR/snapshot"
SETTINGS_DIR="$HOMEBREW_DIR/settings"

export HOMEBREW_BUNDLE_FILE="$BREWFILE"

usage() {
  echo "Usage: $0 {dump|check|apply}"
  exit 1
}

require_brew() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "Error: brew not found in PATH" >&2
    exit 1
  fi
}

cmd_dump() {
  require_brew
  mkdir -p "$SNAPSHOT_DIR"

  echo "[dump] Writing Brewfile from current install..."
  brew bundle dump --file="$BREWFILE" --force --describe

  echo "[dump] Writing snapshot files..."
  brew config > "$SNAPSHOT_DIR/brew-config.txt" || true
  brew --env > "$SNAPSHOT_DIR/brew-env.txt" || true
  brew tap > "$SNAPSHOT_DIR/taps.txt" || true
  brew list --versions > "$SNAPSHOT_DIR/formulae-versions.txt" || true
  brew services list > "$SNAPSHOT_DIR/services.txt" || true

  echo "[dump] Done."
}

cmd_check() {
  require_brew
  echo "[check] Diffing installed packages against Brewfile..."
  brew bundle check --file="$BREWFILE" --verbose
}

cmd_apply() {
  require_brew
  read -rp "This will install/upgrade packages from $BREWFILE. Continue? [y/N] " confirm
  if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
    echo "[apply] Aborted."
    exit 0
  fi
  echo "[apply] Installing Brewfile..."
  brew bundle install --file="$BREWFILE"
  echo "[apply] Done."
}

main() {
  case "${1:-}" in
    dump) cmd_dump ;;
    check) cmd_check ;;
    apply) cmd_apply ;;
    *) usage ;;
  esac
}

main "$@"
