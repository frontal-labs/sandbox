#!/bin/sh
# Builds the Tier 2 guest image (vmlinuz and initrd.gz) for the host's own architecture, in a
# container, and writes it to a host directory. On Apple Silicon that is an aarch64 guest, which
# QEMU runs under HVF. It needs no WASI SDK, so unlike docker/run-vm-tests.sh it works on arm64.
#
#   docker/build-guest-image.sh OUT_DIR
#
# GUEST_PROGRAMS names the programs the image carries (default "ffmpeg chromium"). GUEST_SUFFIX
# is appended to the builder image and cache volume names, so concurrent builders do not share
# them.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:?usage: docker/build-guest-image.sh OUT_DIR}
SUFFIX=${GUEST_SUFFIX:+-$GUEST_SUFFIX}
IMAGE="frontal-guest-builder$SUFFIX"
CACHE="frontal-guest-cache$SUFFIX"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

docker build -q -t "$IMAGE" -f "$ROOT/docker/guest-builder.Dockerfile" "$ROOT/docker" >/dev/null

docker run --rm --user 0:0 \
  -e GUEST_PROGRAMS="${GUEST_PROGRAMS:-ffmpeg chromium}" \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
  -e OUT=/out \
  -v "$ROOT:/src" \
  -v "$OUT:/out" \
  -v "$CACHE:/cache" \
  -w /src \
  "$IMAGE" sh /src/docker/emulated-vm/build-image.sh
