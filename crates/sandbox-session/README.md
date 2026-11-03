# Sandbox Session

The `Session` trait and the isolate (Tier 1) backend.

**Status: partial, in-process.** `IsolateBackend` owns a VFS and a shell. `exec` and `run_script` run builtin commands, `fs()` gives the VFS, and `snapshot`, `restore`, and `sleep` work. `VmBackend` and the Auto tier do not exist.

## Public API

Re-exported from `src/lib.rs`: `IsolateBackend`, `Session`, `restore`.

- `trait Session` (`src/session.rs`): `id()`, `tier()`, `exec(ExecRequest)`, `run_script(&str)`, `fs() -> &dyn SessionFs`, `snapshot() -> ContentHash`, `sleep() -> ContentHash`, and `destroy(self: Box<Self>)`. Every async method returns `Result<_, SandboxError>`.
- `trait SessionFs` (re-exported from `sandbox-vfs`): file operations and `watch` (ADR 0005).
- `IsolateBackend::new(SessionConfig)` starts a session. `exec` refuses streaming, timeouts, and an empty argv, rather than ignoring them.
- `snapshot()` returns the committed root. The session keeps running. The server records the root and owns the snapshot (ADR 0004).
- `sleep()` checkpoints the root, as `snapshot` does. The owner drops the session to release it (ADR 0006).
- `checkpoint()` commits the filesystem and returns a `Checkpoint` (root, working directory, environment). The server never writes the environment to disk. `restore_checkpoint` starts a session from one. `restore(root)` starts a session from a root with the default shell state. Blobs live in a process-wide store, which is never freed (ADR 0010).

## Limits

- A snapshot records only the filesystem root, so a session restored from a snapshot starts with fresh shell state. A checkpoint keeps the working directory and the environment in memory. The saved state keeps only the directory.
- Session state is in memory. The server saves checkpoints to disk when it runs with `--data-dir`.
- Exec timeouts and streaming exec are refused (`UNSUPPORTED`) until they are implemented.

## Spec

See [plans/PLAN.md](../../plans/PLAN.md) section 4.8. The VM tier and the streaming `ExecHandle` in the plan are not implemented.

## License

Apache-2.0
