# Sandbox VMM

QEMU driver for Tier 2 microVM execution.

## Status

Implemented. This crate boots and manages real guests.

- `QemuVm` (`src/qemu.rs`) boots a guest with QEMU, with the guest agent as init. It waits until the agent answers a `Ping` before reporting the guest ready, saves and resumes guest state through the QEMU monitor, and kills QEMU when dropped so a session that ends early cannot leave a process behind.
- `VmPool` (`src/pool.rs`) keeps a target number of guests ready. Each is booted and Ping-checked before it is handed out, so a promotion takes a warm guest off the request path. An empty pool boots one on demand.
- Two accelerators: `Accel::Tcg` (software emulation, works everywhere, slow) and `Accel::Kvm` (hardware virtualisation through `/dev/kvm`, at hardware speed).

The Firecracker driver and jailer that used to live here were removed: they were scaffolding (`start_vm` and `Jailer::setup` returned canned `unsupported` errors), and QEMU is the Tier 2 driver. See the ticket T-05 in `TICKETS.md`.

## Overview

Each guest is a QEMU process with a kernel and initrd (`QemuSpec`), the guest agent as init, and the agent reachable on a forwarded port (`GUEST_AGENT_PORT` inside the guest). The host reaches it over TCP to `127.0.0.1` on a port QEMU forwards (`hostfwd`), which is why the emulated and KVM guests use the same code path.

Source: `crates/sandbox-vmm/src/`.

## Public API (from `src/lib.rs`)

- `QemuVm` (`qemu.rs`): `start(&QemuSpec)`, `start_with(&QemuSpec, Option<&Snapshotting>)`, `wait_for_agent(Duration)`, `save_state(&str)`, `stop()`, `agent() -> SocketAddr`.
- `QemuSpec`: `qemu`, `kernel`, `initrd`, `memory_mib`, `accel`, `console`.
- `Accel`: `Tcg` | `Kvm`, with `from_env()` reading `SANDBOX_VM_ACCEL` and `name()`.
- `Snapshotting`: `disk` (a qcow2 that holds the saved state), `monitor` (the QEMU monitor socket), `resume: Option<String>` (a saved-state tag to resume instead of booting).
- `VmPool` (`pool.rs`): `new(QemuSpec, target, boot_timeout)`, `refill()`, `take() -> QemuVm`, `ready()`, `booted()`.
- `GUEST_AGENT_PORT` (`qemu.rs`): the port the guest agent listens on inside the guest.

## Invariants and security assumptions

- Tier 2 is the only tier where a hardware (VM) boundary applies. QEMU with KVM provides a virtualisation boundary; `Accel::Tcg` is software emulation and is not a security boundary.
- The guest is reachable only on loopback through a forwarded port, not on a network interface.
- The VMM stays behind the session boundary; callers should not reach VM handles directly. `crates/sandbox-promote` owns the Tier 1 to Tier 2 handoff.

## Example

Omitted: booting a guest needs a QEMU binary, a kernel, and an initrd with the guest agent. See `crates/sandbox-promote/tests/` for real examples gated on `SANDBOX_VM_IMAGE`.

See the project specification section 4.10 for the target spec.

## License

Apache-2.0
