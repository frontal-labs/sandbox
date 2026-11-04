#!/bin/sh
# Boots the guest image under QEMU's software emulator (TCG, no KVM) and runs the
# guest-protocol test against the agent's forwarded port. Runs inside the Linux dev image.
set -eu

cd /src
OUT=${OUT:-/cache/emulated-vm/out}
sh /src/docker/emulated-vm/build-image.sh

# The host reaches the agent at 127.0.0.1:7000 in this container.
qemu-system-x86_64 \
  -accel tcg -M q35 -m 512 -smp 1 -no-reboot -display none -monitor none \
  -kernel "$OUT/vmlinuz" -initrd "$OUT/initrd.gz" \
  -append "console=ttyS0 rdinit=/init panic=-1" \
  -netdev user,id=net0,hostfwd=tcp:127.0.0.1:7000-:7000 \
  -device virtio-net-pci,netdev=net0 \
  -serial "file:$OUT/console.log" &
QEMU_PID=$!
trap 'kill "$QEMU_PID" 2>/dev/null || true' EXIT

status=0
SANDBOX_GUEST_ADDR=http://127.0.0.1:7000 \
  cargo test -p sandbox-guest --test emulated_guest -- --ignored --nocapture || status=$?

echo "--- guest console ---"
cat "$OUT/console.log" || true
exit "$status"
