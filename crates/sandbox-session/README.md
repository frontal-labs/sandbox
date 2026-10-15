# Sandbox Session

`Session` trait and the isolate (Tier 1) backend for the sandbox system.

**Status: stub.** The trait is defined. `IsolateBackend` is a shell that records an id and tier. `exec` returns an empty handle, `snapshot` and `sleep` return fresh ids without saving state, and `fs()` panics with `unimplemented!()`. `restore` returns `unsupported`. `VmBackend` and the Auto tier do not exist.

## Public API

Re-exported from `src/lib.rs`: `IsolateBackend`, `Session`.

- `trait Session` (`src/session.rs`): `id()`, `tier()`, `exec(ExecRequest) -> ExecHandle`, `fs() -> Box<dyn SessionFs>`, `snapshot()`, `sleep()`, `destroy(self: Box<Self>)`. All async methods return `Result<_, SandboxError>`.
- `trait SessionFs`: `open`, `read`, `write`, `stat`, `readdir`, `mkdir`, `rm`, `rename`, `symlink`, `readlink`, `truncate`, `commit`. Handles are `u64`.
- `ExecHandle` (`src/session.rs`): buffered `stdout`, `stderr`, and `stdin` as `Vec<u8>`, plus `wait()`. This is not the streaming shape in the PLAN.
- `IsolateBackend` (`src/backend.rs`): `new(SessionConfig) -> Result<Self>`. Its `fs()` panics.
- `Session::snapshot()` returns the committed root hash. The session keeps running. The server records the hash and owns the snapshot.
- `restore(root: ContentHash)` (`src/backend.rs`): starts a session whose filesystem is that root. Blobs live in a process-wide store, which is never freed. The caller decides who may use a root.

## Planned design

- `IsolateBackend` wires together `sandbox-shell`, `sandbox-runtime`, `sandbox-wasm`, `sandbox-vfs`, and `sandbox-policy`. Command resolution goes shell, builtin, VFS executable, WASM tool, then `NeedsVm`.
- `VmBackend` proxies to the guest agent over vsock (`sandbox-vmm`, `sandbox-guest`).
- `Auto` starts on the isolate backend and calls `sandbox-promote` on `NeedsVm`.
- The PLAN's streaming `ExecHandle` (`BoxStream` stdout and stderr, bounded stdin, cancellation token) is not implemented.

## Invariants and security assumptions

- The SDK should not depend on the tier. Tier is read-only session metadata.
- Filesystem access must go through capability checks. Not implemented: `fs()` panics today.
- Session state is not durable. `snapshot` and `sleep` do not record anything.

## Example

Not provided. `exec` returns an empty handle, and `fs()` panics, so an example would be misleading.

## Spec

See [PLAN.md](../../PLAN.md) section 4.8.
