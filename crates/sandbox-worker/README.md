# sandbox-worker

Per-tenant worker process for the sandbox system.

## Status

Stub. `Worker::new(SessionConfig)` returns an empty handle, `spawn_session` returns `SandboxError::unsupported`, and `shutdown` returns `Ok(())`. There is no sandboxing setup (seccomp, cgroups, namespaces, `PR_SET_NO_NEW_PRIVS`), no Unix socket listener, and no gRPC service. `tonic` and `prost` are declared as dependencies but no service is defined. The crate also depends on `sandbox-session`, whose `IsolateBackend` is `unimplemented!()`. No tests exist.

## Overview

Intended design: one OS process per tenant that hosts many of that tenant's sessions, sandboxes itself at startup, and serves scheduler RPCs over a Unix socket. Source: `crates/sandbox-worker/src/`.

## Public API (from `src/lib.rs`)

- `Worker` (`worker.rs`): `new(SessionConfig).await`, `spawn_session(SessionConfig).await -> SessionId` (stub), `shutdown().await`, `Default`.

## Invariants and security assumptions

- Target: a worker never holds credentials for another tenant, and a worker crash must not affect other tenants. Not enforced yet.
- Target startup hardening (not implemented): `PR_SET_NO_NEW_PRIVS`, seccomp-bpf allowlist, cgroup v2 limits, optional user/mount/pid/net namespaces, no host mounts.
- Crash recovery is a scheduler responsibility (restore from last snapshot or report failure).

## Example

Omitted: the worker does not serve requests yet.

See `PLAN.md` section 4.13 for the target spec.
