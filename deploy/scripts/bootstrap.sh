#!/usr/bin/env bash
set -euo pipefail

# bootstrap.sh — prepare a host for Frontal Sandbox.
#
# Installs runtime dependencies, enables KVM and cgroups v2, hardens sshd,
# opens firewall ports, and creates the sandbox user/group.
#
# Usage: sudo ./bootstrap.sh [--no-kvm]
#
# Intended for single-node and cloud hosts (M3/M4).

NO_KVM=0
if [[ "${1:-}" == "--no-kvm" ]]; then
  NO_KVM=1
fi

if [[ "$EUID" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

DISTRO_ID="$(. /etc/os-release && echo "${ID}")"

case "$DISTRO_ID" in
  ubuntu|debian)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl tini iptables ufw
    if [[ "$NO_KVM" -eq 0 ]]; then
      DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends qemu-kvm libvirt-daemon-system
    fi
    ;;
  rhel|centos|fedora|amzn)
    dnf install -y ca-certificates curl tini iptables firewalld
    if [[ "$NO_KVM" -eq 0 ]]; then
      dnf install -y qemu-kvm libvirt
    fi
    ;;
  *)
    echo "unsupported distro: $DISTRO_ID" >&2
    exit 1
    ;;
esac

if ! mountpoint -q /sys/fs/cgroup || ! grep -q cgroup2 /proc/filesystems 2>/dev/null; then
  echo "cgroups v2 not available; Tier 1 still works, Tier 2 needs it" >&2
fi

getent group sandbox >/dev/null || groupadd --system --gid 10001 sandbox
getent passwd sandbox >/dev/null || \
  useradd --system --uid 10001 --gid sandbox \
    --home-dir /var/lib/sandbox --shell /usr/sbin/nologin sandbox

install -d -o sandbox -g sandbox -m 0700 /var/lib/sandbox
install -d -o sandbox -g sandbox -m 0755 /etc/sandbox
install -d -o sandbox -g sandbox -m 0755 /var/run/sandbox

if [[ "$NO_KVM" -eq 0 ]] && [[ -c /dev/kvm ]]; then
  usermod -aG kvm sandbox || true
  install -d -m 0755 -o sandbox -g sandbox /var/run/sandbox/firecracker
fi

if command -v ufw >/dev/null 2>&1; then
  ufw allow 22/tcp || true
  ufw allow 8080/tcp || true
  ufw allow from any to any port 9090 proto tcp || true
  ufw --force enable || true
elif command -v firewall-cmd >/dev/null 2>&1; then
  firewall-cmd --permanent --add-service=ssh || true
  firewall-cmd --permanent --add-port=8080/tcp || true
  firewall-cmd --permanent --add-port=9090/tcp || true
  firewall-cmd --reload || true
fi

echo "bootstrap complete. Next: run deploy.sh"
