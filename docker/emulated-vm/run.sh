#!/bin/sh
# Runs inside the Linux dev image (as root). Builds the guest agent, packs it into an
# initramfs with a Debian kernel, boots that under QEMU's software emulator (TCG, no
# KVM), and runs the guest-protocol test against the agent's forwarded TCP port.
#
# The VM's init starts the agent as PID 1's child, so the agent runs under a booted
# kernel. The console log is kept and printed, so a failed boot shows its cause.
set -eu

cd /src
cargo build -q -p sandbox-guest --bin sandbox-guest-agent
AGENT="$CARGO_TARGET_DIR/debug/sandbox-guest-agent"

KVER=$(ls /lib/modules | head -n 1)
KERNEL="/boot/vmlinuz-$KVER"
W=/cache/emulated-vm
rm -rf "$W"
mkdir -p "$W/root/bin" "$W/root/sbin" "$W/root/dev" "$W/root/proc" "$W/root/sys" "$W/root/run" "$W/root/tmp"

# Busybox provides the init shell and the few applets it needs.
cp /bin/busybox "$W/root/bin/busybox"
for applet in sh mount mkdir ip insmod uname mknod cat ln sleep; do
  ln -sf busybox "$W/root/bin/$applet"
done

# The agent, and the shared libraries it links against.
# Stripped: the debug build carries its symbols, which would fill the initramfs.
strip -o "$W/root/sbin/sandbox-guest-agent" "$AGENT"
ldd "$AGENT" | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^\//) print $i }' | sort -u |
  while read -r lib; do
    mkdir -p "$W/root$(dirname "$lib")"
    cp -L "$lib" "$W/root$lib"
  done

# The virtio network driver and its dependencies, decompressed for busybox insmod.
# modprobe prints only the first name's chain when given several, so each name is resolved
# on its own. Dependencies come first, and duplicates are dropped in order.
for name in virtio_pci virtio_net; do
  modprobe -S "$KVER" --show-depends "$name"
done | awk '$1 == "insmod" && !seen[$2]++ { print $2 }' > "$W/modules.src"
: > "$W/modules.list"
while read -r module; do
  path="${module%.xz}"
  mkdir -p "$(dirname "$W/root$path")"
  case "$module" in
    *.xz) xz -dc "$module" > "$W/root$path" ;;
    *) cp "$module" "$W/root$path" ;;
  esac
  echo "$path" >> "$W/modules.list"
done < "$W/modules.src"
cp "$W/modules.list" "$W/root/modules.list"

mknod -m 600 "$W/root/dev/console" c 5 1
mknod -m 666 "$W/root/dev/null" c 1 3

cat > "$W/root/init" <<'INIT'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
echo "init: booted kernel $(uname -r)"
while read -r module; do
  insmod "$module" && echo "init: loaded $module" || echo "init: could not load $module"
done < /modules.list
# The NIC appears when its driver binds, which can lag the module load.
waited=0
while [ ! -e /sys/class/net/eth0 ] && [ "$waited" -lt 20 ]; do
  sleep 1
  waited=$((waited + 1))
done
echo "init: network devices: $(ls /sys/class/net | tr '\n' ' ')"
ip link set lo up
ip link set eth0 up
ip addr add 10.0.2.15/24 dev eth0
ip route add default via 10.0.2.2
echo "init: starting the guest agent"
exec /sbin/sandbox-guest-agent --tcp 0.0.0.0:7000
INIT
chmod +x "$W/root/init"

(cd "$W/root" && find . | cpio -o -H newc --quiet | gzip -1) > "$W/initrd.gz"

# The host reaches the agent at 127.0.0.1:7000 in this container.
qemu-system-x86_64 \
  -accel tcg -M q35 -m 512 -smp 1 -no-reboot -display none -monitor none \
  -kernel "$KERNEL" -initrd "$W/initrd.gz" \
  -append "console=ttyS0 rdinit=/init panic=-1" \
  -netdev user,id=net0,hostfwd=tcp:127.0.0.1:7000-:7000 \
  -device virtio-net-pci,netdev=net0 \
  -serial "file:$W/console.log" &
QEMU_PID=$!
trap 'kill "$QEMU_PID" 2>/dev/null || true' EXIT

status=0
SANDBOX_GUEST_ADDR=http://127.0.0.1:7000 \
  cargo test -p sandbox-guest --test emulated_guest -- --ignored --nocapture || status=$?

echo "--- guest console ---"
cat "$W/console.log" || true
exit "$status"
