# Sandbox VMM

Firecracker VMM driver for Tier 2 microVM execution.

## Status

Scaffolding. Nothing boots a VM yet.

- `VmmDriver::start_vm` returns `SandboxError::unsupported("VMM not implemented")`.
- `Jailer::setup` returns `unsupported("Jailer not implemented")`.
- `VmPool` (`src/pool.rs`) is an in-memory FIFO of `VmHandle` values: `acquire` pops one (or `None`), `release` pushes one back, `stats` reports counts. It does not create VMs, has no min/max sizing logic, and has no per-image keying.
- There are two different `VmHandle` types (`driver.rs` with `vsock_addr`, `pool.rs` with `image`). They need to be unified.
- No Firecracker REST client, guest-agent protocol, snapshot/restore, or networking exists in this crate. No tests exist.

## Overview

Intended to drive Firecracker over its Unix-socket REST API, with a jailer per VM (chroot, cgroups, netns, seccomp, dropped capabilities). Source: `crates/sandbox-vmm/src/`.

## Public API (from `src/lib.rs`)

- `VmmDriver` (`driver.rs`): `new()`, `start_vm(&SessionConfig)` (stub). `VmHandle { id, vsock_addr }`.
- `Jailer` (`jailer.rs`): `new()`, `setup(&SessionConfig)` (stub).
- `VmPool`, `PoolConfig { min_size, max_size }`, `PoolStats` (`pool.rs`): see Status.

## Invariants and security assumptions

- Tier 2 is the only tier where a hardware (VM) boundary applies. Until the jailer is implemented, the VMM provides no isolation.
- Per-VM jailing (no host mounts, read-only binary, seccomp allowlist, cgroup limits) is the required design, not a current property.
- The VMM stays behind the session boundary; callers should not reach VM handles directly.

## Example

Omitted: the driver and pool do not start real VMs yet.

See `plans/PLAN.md` section 4.10 for the target spec.
