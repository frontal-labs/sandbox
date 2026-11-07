#!/usr/bin/env bash
set -euo pipefail

# build.sh — compile every Bazel target and the release CLI binary.
#
# Produces artifacts consumed by the test and deploy stages:
#   - bazel-out/ (Bazel build cache)
#   - target/release/frontal-sandbox (CLI binary, used by deploy)
#   - dist/ (TypeScript SDK package artifacts)
#
# Runnable locally: `./.buildkite/scripts/build.sh`.

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

echo "build: Bazel //..."
bazel build //...

echo "build: release CLI binary"
cargo build --release -p cli --bin frontal-sandbox

echo "build: TypeScript SDK"
bun install --frozen-lockfile
bun run ts:build

echo "build: Docker image (smoke test of the compose path)"
docker build -f deploy/Dockerfile -t frontal-sandbox:build .

echo "build: done"