# Homebrew config for the Sandbox CLI

Everything Homebrew for the `sandbox` CLI lives in this directory:

- `Formula/sandbox.rb` is the formula. It is generated, so do not edit it by hand.
- `render-formula.sh` writes the formula for a release.

The formula installs the `sandbox` binary from the GitHub release binaries that
`.github/workflows/release-binaries.yml` attaches to each `v*` tag. It covers
macOS and Linux on arm64 and x86_64. Windows has no formula.

## Layout and publishing

This directory is named `homebrew` so that it can become a tap. Homebrew
expects a tap to be a repo whose root holds `Formula/`, and it cannot tap a
subdirectory of another repo. So the directory has to be published as its own
repo, `frontal-labs/homebrew`, before anyone can run `brew tap`. Split it out
with:

```bash
git subtree split --prefix homebrew -b homebrew-split
git push git@github.com:frontal-labs/homebrew.git homebrew-split:main
```

Once that repo exists, users run:

```bash
brew tap frontal-labs/cli
brew install frontal-labs/cli/sandbox
```

Until then, `brew install` from this repo does not work.

## Render a release

```bash
homebrew/render-formula.sh 0.1.1
```

The script downloads the four Unix binaries for `v0.1.1`, records their SHA-256
digests, and rewrites `Formula/sandbox.rb`. It fails if any binary is missing,
which happens when the release workflow has not finished for that tag. Commit
the result and republish the split.

## Check a formula

```bash
brew style homebrew/Formula/sandbox.rb
brew tap-new --no-git local/check && cp homebrew/Formula/sandbox.rb "$(brew --repo local/check)/Formula/"
brew audit --strict --online local/check/sandbox
brew untap local/check
```

`brew audit` takes a tap-qualified name, not a file path, which is why the check
copies the formula into a throwaway tap first.

## Machine setup

This directory also holds version-controlled Homebrew configuration for this
machine (macOS 26.6.2 arm64, prefix /opt/homebrew, Homebrew 7.0.9).

- `Brewfile` — taps, formulae, casks (source of truth, from `brew bundle dump`).
- `settings/homebrew.env.example` + `settings/env-reference.md` — `HOMEBREW_*`
  variables (names only, no secrets).
- `snapshot/` — brew config, `brew --env`, taps, formulae versions, services,
  analytics/developer state.
- `bin/brew-sync.sh` — dump / check / apply modes around `brew bundle`.

Usage:

```bash
homebrew/bin/brew-sync.sh dump   # regenerate Brewfile + snapshot
homebrew/bin/brew-sync.sh check  # diff install vs Brewfile
homebrew/bin/brew-sync.sh apply  # install Brewfile after confirmation
```

Target this folder directly with:

```bash
HOMEBREW_BUNDLE_FILE=homebrew/Brewfile brew bundle <cmd>
```

### Known issue

`stainless-api/tap` is untrusted, so its cask (`stl`) fails to load;
`snapshot/casks-versions.txt` is excluded. Trust it via
`brew trust --tap stainless-api/tap` if desired.

### Secrets

None committed; tokens live in keychain/CI secret store.

### Notable

`Brewfile` includes `coder/coder/coder-desktop` with `trusted: true`; Mise
(activated via `~/.zshrc`) is not managed here.

## Formula publishing

- The release assets must be publicly downloadable. The `frontal-labs/sandbox`
  repo is public, so `brew install` needs no token.
- Each binary is a bare executable, not an archive, so the formula uses
  `using: :nounzip` and renames the file to `sandbox` on install.
