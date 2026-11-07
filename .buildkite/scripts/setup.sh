#!/usr/bin/env bash
set -euo pipefail

# setup.sh — prepare the agent for the Frontal Sandbox build.
#
# Installs the toolchains the repository pins and warms the caches that CI
# depends on. Runnable locally: `./.buildkite/scripts/setup.sh`.
#
#   - Bazelisk (reads .bazelversion)
#   - Rust 1.95 with rustfmt + clippy, plus the cross targets the release
#     pipeline builds for
#   - protoc (sandbox-proto's build script runs it)
#   - Node 22 (TypeScript SDK)
#   - Python 3.12 (Python SDK)

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "setup: installing Bazelisk"
curl -fsSL "https://bazel.build/bazelisk/releases/latest/bazelisk-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)" \
  -o /usr/local/bin/bazelisk \
  && chmod +x /usr/local/bin/bazelisk \
  && ln -sf /usr/local/bin/bazelisk /usr/local/bin/bazel

echo "setup: installing Rust 1.95"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
  --profile minimal \
  --default-toolchain 1.95.0 \
  --component rustfmt \
  --component clippy \
  --target aarch64-apple-darwin \
  --target x86_64-apple-darwin \
  --target x86_64-unknown-linux-gnu
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

echo "setup: installing protoc"
PROTOC_VERSION=29.3
case "$(uname -s)" in
  Linux)   OS=linux ;;
  Darwin)  OS=darwin ;;
  *) echo "setup: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64)  ARCH=x86_64 ;;
  arm64|aarch64) ARCH=aarch64 ;;
  *) echo "setup: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac
curl -fsSL "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-${OS}-${ARCH}.zip" \
  -o /tmp/protoc.zip
python3 -c "import zipfile; zipfile.ZipFile('/tmp/protoc.zip').extractall('/usr/local')"
protoc --version

echo "setup: installing Node 22"
if command -v nvm >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "$HOME/.nvm/nvm.sh"
fi
nvm install 22 >/dev/null || true
nvm use 22 >/dev/null || true
node --version
npm --version

echo "setup: installing Python 3.12"
if ! command -v python3.12 >/dev/null 2>&1; then
  case "$(uname -s)" in
    Linux)
      sudo apt-get update -qq \
        && sudo apt-get install -y -qq python3.12 python3.12-venv python3-pip
      ;;
    Darwin)
      brew install python@3.12
      ;;
  esac
fi
python3.12 --version

echo "setup: caching Bazel"
bazel --version
# Warm the external repositories and download the Rust toolchain so the first
# build step does not pay for it.
bazel build //... --disk_cache=/var/cache/bazel 2>/dev/null \
  || echo "setup: bazel warm build skipped (expected on a fresh agent)"

echo "setup: done"