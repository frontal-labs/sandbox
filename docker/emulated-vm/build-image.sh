#!/bin/sh
# Builds the guest image for the emulated VM: the guest agent as init, a Debian kernel,
# and an initramfs holding the agent's shared libraries, busybox, and the virtio network
# modules. Runs inside the Linux dev image as root. Writes $OUT/vmlinuz and $OUT/initrd.gz.
# The agent listens on TCP port 7000 inside the guest.
set -eu

cd /src
cargo build -q -p sandbox-guest --bin sandbox-guest-agent
AGENT="$CARGO_TARGET_DIR/debug/sandbox-guest-agent"

KVER=$(ls /lib/modules | head -n 1)
KERNEL="/boot/vmlinuz-$KVER"
OUT=${OUT:-/cache/emulated-vm/out}
W=/cache/emulated-vm/work
rm -rf "$W"; mkdir -p "$OUT"
mkdir -p "$W/root/bin" "$W/root/sbin" "$W/root/dev" "$W/root/proc" "$W/root/sys" "$W/root/run" "$W/root/tmp"

# Busybox provides the init shell and the few applets it needs.
cp /bin/busybox "$W/root/bin/busybox"
for applet in sh mount mkdir ip insmod uname mknod cat ln sleep; do
  ln -sf busybox "$W/root/bin/$applet"
done

# The agent, and the shared libraries it links against.
# Stripped: the debug build carries its symbols, which would fill the initramfs.
strip -o "$W/root/sbin/sandbox-guest-agent" "$AGENT"
# Programs the image carries beyond busybox, each at its own place under /usr/bin, with the
# shared libraries it links. GUEST_PROGRAMS names them: the dev image installs them.
copy_libraries() {
  ldd "$1" | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^\//) print $i }' | sort -u |
    while read -r lib; do
      mkdir -p "$W/root$(dirname "$lib")"
      cp -L "$lib" "$W/root$lib"
    done
}
copy_libraries "$AGENT"
mkdir -p "$W/root/usr/bin"
for program in ${GUEST_PROGRAMS:-ffmpeg}; do
  src=$(command -v "$program") || { echo "guest program $program is not installed" >&2; exit 1; }
  cp -L "$src" "$W/root/usr/bin/$program"
  copy_libraries "$src"
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

(cd "$W/root" && find . | cpio -o -H newc --quiet | gzip -1) > "$OUT/initrd.gz"
cp "$KERNEL" "$OUT/vmlinuz"
echo "guest image: $OUT/vmlinuz $OUT/initrd.gz"

