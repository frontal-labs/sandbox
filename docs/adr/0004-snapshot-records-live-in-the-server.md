# 0004: Snapshot records live in the server

## Status
Accepted

## Date
2026-10-15

## Context
The session crate kept a process-wide registry of snapshot IDs and their roots. Snapshots are owned by API keys, and the owner checks and limits belong to the server. With the registry in the session crate, one key's snapshots shared a global cap with every other key, and the server could not check ownership against the record.

## Decision
- `Session::snapshot()` returns the committed root hash (`ContentHash`), not a `SnapshotId`. The session crate no longer assigns or stores IDs.
- `sandbox_session::restore` takes a `ContentHash`. The caller decides which roots it may restore.
- `sandbox-server` owns the snapshot table: each record has an owner key, the source session, the root, the creation time, and the labels. The table enforces a per-key limit (32), a total limit (4096), and owner checks on fork, on `initial_snapshot`, and on the two GET routes.
- The blob store stays process-wide. Blobs are content addressed and immutable, so sharing them does not leak data across keys.

## Consequences
### Positive
- The limits and the owner checks use the same records, in one module.
- A key's snapshots cannot be read or restored by another key, because the server checks the record's owner before it touches the root.

### Negative
- The total cap of 4096 is still shared across keys. Enough keys can fill it, which denies snapshots to everyone. Per-key limits stop one key from doing that alone. Fully isolating the total would need per-key quotas that sum to less than the total, which is a policy decision that is not made yet.
- `Session` is a public trait, so the change to `snapshot`'s return type breaks implementers. There is one implementation, `IsolateBackend`.
- Snapshots capture only the filesystem. `env` is empty and `cwd` is `/` in the API view, because the shell's state is not captured yet.
