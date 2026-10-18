# Sandbox Promote

Tier 1 (isolate) to Tier 2 (VM) promotion for the sandbox system.

**Status: stub.** `PromoteManager::promote_session` always returns an `unsupported` error. No VFS commit, block image export, VM acquisition, or backend swap happens.

## Public API

Re-exported from `src/lib.rs`:

- `PromoteManager` (`src/promote.rs`)
  - `async fn promote_session(session: Arc<dyn Session>, config: SessionConfig) -> Result<(), SandboxError>`: currently returns `Err(unsupported)`.
  - `Default` implementation.

## Planned flow

The PLAN describes this sequence. None of it is implemented:

1. Pause the isolate session.
2. `vfs.commit()` to a new root hash.
3. `export_ext4(root)` to a block image, cached by root hash.
4. Acquire a VM from the pool and attach the image.
5. Transfer env, cwd, and limits, then re-run the failed command in the VM.
6. Swap the session backend and emit a `Promoted` event.

Planned properties:

- Running processes do not survive promotion. The failed command is re-run.
- Reverse sync (VM to VFS) happens only on explicit `snapshot()` or `sleep()`.
- An SDK `promote_hint` lets users request Tier 2 up front.

## Security assumptions (planned)

- Promotion must not give the VM any host credentials or host paths beyond the exported block image.

## Spec

See [plans/PLAN.md](../../plans/PLAN.md) section 4.11. Performance target: promotion under 400 ms.
