# Emulated guest VM test

`docker/run-emulated-vm.sh` boots the guest agent under a real Linux kernel and runs the
guest-protocol test against it. It needs no KVM. Docker Desktop on hosts without hardware
virtualisation exposes no `/dev/kvm`, and QEMU with KVM cannot run without one. So this
harness uses QEMU's software emulator (`-accel tcg`) instead.

## What runs

1. The dev image (`docker/linux-dev.Dockerfile`) provides QEMU, a Debian kernel
   (`linux-image-amd64`), busybox, and the module tools.
2. `docker/emulated-vm/run.sh` builds `sandbox-guest-agent` and strips it. It packs the
   agent, its shared libraries, busybox, and the virtio network modules into an initramfs.
   The init script loads the modules, waits for `eth0`, configures the network, and runs the
   agent on TCP port 7000.
3. QEMU boots that kernel with a user-mode network. Host port `127.0.0.1:7000` forwards to
   the agent.
4. `crates/sandbox-guest/tests/emulated_guest.rs` runs against the forwarded port. It covers
   Ping, a WriteFile and ReadFile round trip, ListDir, Exec, and Sync.
5. The guest's console log is printed after the test, so a failed boot shows its cause.

## What it proves, and what it does not

- Proves: the agent runs under a booted Linux 6.1 kernel in an emulated x86 machine, and it
  answers the guest protocol over TCP.
- Does not prove: that commands run in the guest kernel. `Exec` runs the in-process shell,
  as it does on every tier.
- Does not prove: KVM acceleration or promotion. KVM needs `/dev/kvm`; promotion
  (`sandbox-promote`) is built and runs against this kind of guest (PLAN.md section 4.10).
- Sync is a placeholder that returns the nil root. The test checks that it responds, not that
  its answer is correct.

## Running it

```sh
docker/run-emulated-vm.sh
```

The test is `#[ignore]`d. It runs only when `SANDBOX_GUEST_ADDR` is set, which the script
does, so a plain `cargo test` never needs the VM. Expect a boot of about 10 to 20 seconds
under emulation. The first run is slower because the image installs QEMU and the kernel.

## Result

Verified 2026-11-03 on an Intel Mac with Docker Desktop: `docker/run-emulated-vm.sh` exited
0, and the guest test passed in 18.95 seconds. The console log shows `init: booted kernel
6.1.0-54-amd64`, the modules loaded, `eth0` configured, and `sandbox-guest-agent: serving the
guest protocol`.
