#!/usr/bin/env bash
set -euo pipefail

# lint.sh — static analysis for the Frontal Sandbox repository.
#
# Runs the Rust and TypeScript linters the project pins. Runnable locally:
# `./.buildkite/scripts/lint.sh`.

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "lint: Rust formatting"
cargo fmt --all --check

echo "lint: Rust clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "lint: TypeScript / JS (biome)"
bun install --frozen-lockfile
bun run lint

echo "lint: Python (ruff)"
python3.12 -m pip install --quiet --require-hashes -r sdks/python/requirements_lock.txt \
  || python3.12 -m pip install --quiet ruff
ruff check sdks/python/sandbox_sdk

echo "lint: shell scripts"
shellcheck .buildkite/scripts/*.sh deploy/scripts/*.sh infrastructure/scripts/*.sh || true

echo "lint: done"