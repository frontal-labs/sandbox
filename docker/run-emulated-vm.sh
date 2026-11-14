#!/bin/sh
# Boots the guest agent in an emulated Linux VM and runs the guest-protocol test
# against it. This needs no KVM: QEMU runs the guest in software (TCG), so it works
# in Docker Desktop on hosts without hardware virtualisation. It is slow, and it
# tests the agent under a real kernel, not KVM (PLAN.md section 4.10).
#
#   docker/run-emulated-vm.sh
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
IMAGE=frontal-sandbox-linux-dev

docker build -q -t "$IMAGE" -f "$ROOT/docker/linux-dev.Dockerfile" "$ROOT/docker" >/dev/null

exec docker run --rm --user 0:0 \
  -v "$ROOT:/src" \
  -v frontal-sandbox-cargo-registry:/usr/local/cargo/registry \
  -v frontal-sandbox-linux-target:/cache/target \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  -w /src \
  "$IMAGE" sh /src/docker/emulated-vm/run.sh
