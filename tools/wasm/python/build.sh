#!/bin/sh
# Builds CPython 3.13.1 for wasm32-wasip1 and packs its stdlib. Run it in the Linux dev
# image, which has the pinned WASI SDK (docker/linux-dev.Dockerfile):
#
#   tools/wasm/python/build.sh        (from the repository root, on a Docker host)
#
# The source tarball and the WASI SDK are pinned by SHA-256. The output lands in
# tools/wasm/python/out/: python.wasm (stripped), python313.zip (the stdlib without
# tests), and the BLAKE3 hashes of both. Nothing is installed into the sandbox here.
# That needs the WASI host work tracked in docs/PYTHON_WASM.md.
set -eu

PY_VERSION=3.13.1
PY_SHA256=9cf9427bee9e2242e3877dd0f6b641c1853ca461f39d6503ce260a59c80bf0d9
WIZER_VERSION=10.0.0
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
IMAGE=frontal-sandbox-linux-dev
CACHE=frontal-cpython-cache

docker build -q -t "$IMAGE" -f "$ROOT/docker/linux-dev.Dockerfile" "$ROOT/docker" >/dev/null

exec docker run --rm --user 0:0 \
  -v "$ROOT/tools/wasm/python:/stage:ro" \
  -v "$ROOT/tools/wasm/python/out:/out" \
  -v "$CACHE:/cache" \
  -e PY_VERSION="$PY_VERSION" -e PY_SHA256="$PY_SHA256" -e WIZER_VERSION="$WIZER_VERSION" \
  -e WASI_SDK_PATH=/opt/wasi-sdk \
  "$IMAGE" sh /stage/inside.sh
