# Homebrew config for the Sandbox CLI

Everything Homebrew for the `sandbox` CLI lives in this directory:

- `Formula/sandbox.rb` is the formula. It is generated, so do not edit it by hand.
- `render-formula.sh` writes the formula for a release.

The formula installs the `sandbox` binary from the GitHub release binaries that
`.github/workflows/release-binaries.yml` attaches to each `v*` tag. It covers
macOS and Linux on arm64 and x86_64. Windows has no formula.

## Layout and publishing

This directory is named `homebrew-cli` so that it can become a tap. Homebrew
expects a tap to be a repo whose root holds `Formula/`, and it cannot tap a
subdirectory of another repo. So the directory has to be published as its own
repo, `frontal-labs/homebrew-cli`, before anyone can run `brew tap`. Split it out
with:

```bash
git subtree split --prefix homebrew-cli -b homebrew-cli-split
git push git@github.com:frontal-labs/homebrew-cli.git homebrew-cli-split:main
```

Once that repo exists, users run:

```bash
brew tap frontal-labs/cli
brew install frontal-labs/cli/sandbox
```

Until then, `brew install` from this repo does not work.

## Render a release

```bash
homebrew-cli/render-formula.sh 0.1.1
```

The script downloads the four Unix binaries for `v0.1.1`, records their SHA-256
digests, and rewrites `Formula/sandbox.rb`. It fails if any binary is missing,
which happens when the release workflow has not finished for that tag. Commit
the result and republish the split.

## Check a formula

```bash
brew style homebrew-cli/Formula/sandbox.rb
brew tap-new --no-git local/check && cp homebrew-cli/Formula/sandbox.rb "$(brew --repo local/check)/Formula/"
brew audit --strict --online local/check/sandbox
brew untap local/check
```

`brew audit` takes a tap-qualified name, not a file path, which is why the check
copies the formula into a throwaway tap first.

## Constraints

- The release assets must be publicly downloadable. The `frontal-labs/sandbox`
  repo is public, so `brew install` needs no token.
- Each binary is a bare executable, not an archive, so the formula uses
  `using: :nounzip` and renames the file to `sandbox` on install.
