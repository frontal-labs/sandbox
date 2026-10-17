# Homebrew environment variable reference

These are the `HOMEBREW_*` variables used by this machine's Homebrew setup.
See `homebrew.env.example` for a copyable template.

## Variables

- `HOMEBREW_PREFIX` — Homebrew installation prefix (`/opt/homebrew` on Apple Silicon).
- `HOMEBREW_NO_ANALYTICS` — Disable Homebrew analytics (`1` to disable).
- `HOMEBREW_NO_AUTO_UPDATE` — Disable automatic updates before commands (`1` to disable).
- `HOMEBREW_NO_INSTALL_CLEANUP` — Skip `brew cleanup` after install/upgrade (`1` to skip).
- `HOMEBREW_GITHUB_API_TOKEN` — GitHub personal access token for higher API rate limits (optional; stored in keychain/CI, never committed).
- `HOMEBREW_CASK_OPTS` — Default options passed to `brew install --cask` (e.g. `--no-quarantine`).

## Notes

- No secrets are committed to this repository.
- Tokens live in the macOS keychain or CI secret store.
- Source `homebrew.env` from your shell rc (e.g. `~/.zshrc`) to apply these variables.
