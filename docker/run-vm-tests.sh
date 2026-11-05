#!/bin/sh
# Builds the guest image and runs the VM tests under QEMU's software emulator. No KVM is
# needed: QEMU runs the guest in software (TCG). Runs in the Linux dev image as root.
#
#   docker/run-vm-tests.sh [cargo test arguments...]
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
IMAGE=frontal-sandbox-linux-dev

docker build -q -t "$IMAGE" -f "$ROOT/docker/linux-dev.Dockerfile" "$ROOT/docker" >/dev/null

# With /dev/kvm on the host (a KVM-enabled runner), the container gets it, and the tests
# run the guest with hardware virtualisation. Without it, QEMU emulates the guest.
kvm=""
if [ -e /dev/kvm ]; then
  kvm="--device /dev/kvm"
  export SANDBOX_VM_ACCEL=${SANDBOX_VM_ACCEL:-kvm}
fi

exec docker run --rm --user 0:0 $kvm \
  -e SANDBOX_VM_ACCEL="${SANDBOX_VM_ACCEL:-tcg}" \
  -e SANDBOX_PYTHON_DIR=/src/tools/wasm/python/out \
  -v "$ROOT:/src" \
  -v frontal-sandbox-cargo-registry:/usr/local/cargo/registry \
  -v frontal-sandbox-linux-target:/cache/target \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  -e SANDBOX_VM_IMAGE=/cache/emulated-vm/out \
  -w /src \
  "$IMAGE" sh -c '
    set -eu
    GUEST_PROGRAMS="ffmpeg chromium" sh /src/docker/emulated-vm/build-image.sh >/dev/null
    cargo test -p sandbox-vmm --test qemu_guest -- --ignored --nocapture "$@"
    cargo test -p sandbox-promote --test into_guest -- --ignored --nocapture "$@"
    cargo test -p sandbox-promote --test vm_session -- --ignored --nocapture "$@"
    cargo test -p sandbox-server --test vm_fallback -- --ignored --nocapture "$@"
    cargo test -p sandbox-promote --test real_programs -- --ignored --nocapture "$@"
    cargo test -p sandbox-vmm --test warm_pool -- --ignored --nocapture "$@"
    cargo test -p sandbox-promote --test python_in_guest -- --ignored --nocapture "$@"
    cargo test -p sandbox-promote --test timings -- --ignored --nocapture "$@"
  ' sh "$@"
