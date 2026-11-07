#!/usr/bin/env bash
set -euo pipefail

# test.sh — run the full test suite and emit JUnit XML.
#
# The JUnit files are written to target/junit/ so Buildkite's Annotate plugin
# can collect them via artifact_paths. Runnable locally:
# `./.buildkite/scripts/test.sh`.

export RUSTY_V8_MIRROR="https://github.com/denoland/rusty_v8/releases/download"
export RUSTY_V8_MIRROR_FALLBACK="1"
export V8_FROM_SOURCE="0"

mkdir -p target/junit

echo "test: Rust unit + integration"
cargo test --workspace --junit target/junit/rust.xml

echo "test: Bazel test suite (unit + integration)"
bazel test //... --test_output=errors

echo "test: TypeScript SDK"
bun install --frozen-lockfile
mkdir -p sdks/typescript/junit
bun test --reporter=junit --reporter-file=sdks/typescript/junit/typescript.xml \
  --workspace @frontal-labs/sandbox-sdk \
  || bun test --workspace @frontal-labs/sandbox-sdk

echo "test: Python SDK"
python3.12 -m venv .venv
# shellcheck disable=SC1091
source .venv/bin/activate
python3.12 -m pip install --quiet -e "sdks/python[dev]"
python3.12 -m pytest sdks/python/tests -q \
  --junitxml=target/junit/python.xml

echo "test: done"