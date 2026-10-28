# 0006: Sleep and wake

## Status
Accepted

## Date
2026-10-15

## Context
`Session::sleep` returned `Unsupported`. plans/PLAN.md calls for sleeping sessions that hold no memory, and the sleep method was declared to return a `SnapshotId`. ADR 0004 moved snapshot IDs to the server, so the session crate can no longer return one.

## Decision
- `Session::sleep()` returns the committed root (`ContentHash`), the same type as `snapshot()`. For `IsolateBackend`, sleep is a checkpoint. The session keeps running until its owner drops it.
- `POST /v1/sessions/{id}/sleep` checkpoints the session, records the root as a snapshot under the caller's key, and releases the live session. The response is `{ "snapshot_id" }`. The session's status becomes `sleeping`. The snapshot is an ordinary snapshot, so it can be listed and forked.
- The server keeps the root to wake from in the session's entry. The live session is dropped, so the VFS and its overlay are freed.
- `POST /v1/sessions/{id}/wake` restores a session from that root and returns the session state with status `running`. The session keeps its ID.
- While asleep, exec, file operations, and snapshot return `INVALID_STATE` (HTTP 409). *Superseded by ADR 0015: these requests now wake the session first.* Get and destroy still work without waking. Sleep and wake in the wrong state return 409.
- Sleep takes the session's gate and the snapshot admission lock. It counts against the per-key and total snapshot limits, and it is refused when a limit is reached. In that case the session is not released.
- Shell state (working directory and environment) is not part of the root, so a woken session starts with fresh shell state. ADR 0010 supersedes this: sleep now keeps the working directory and environment.
- Idle sleep, where the scheduler sleeps sessions by itself, is not implemented. Sleep and wake are explicit calls. *Superseded by ADR 0015, which adds idle sleep.*

## Consequences
### Positive
- A sleeping session holds no VFS in memory.
- Sleep reuses the snapshot path, so limits, ownership checks, and forks apply to sleep snapshots without new code.

### Negative
- `Session::sleep` changes its return type. The only implementer is `IsolateBackend`.
- The root is held in the server's memory, so a server restart loses sleeping sessions. The snapshot record is held there too (ADR 0004), so the roots are not durable yet.
- A woken session loses its shell state. Superseded by ADR 0010.
