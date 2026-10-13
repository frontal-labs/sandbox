# sandbox-snapshot

Snapshot, fork, and restore for the sandbox system.

**Status: early, in-memory.** `SnapshotManager` stores snapshot records in a `HashMap` behind an `RwLock`. Nothing is written to the CAS. `take_snapshot` records a fixed `cwd` of `/work` and an empty `env`. `fork_snapshot` creates `SessionState` records but does not create sessions or overlays. Snapshots are lost on restart. No tests exist yet.

## Public API

Re-exported from `src/lib.rs`: `SnapshotManager`.

- `SnapshotManager` (`src/snapshot.rs`)
  - `new()`, and `Default`. No CAS or VFS parameters.
  - `async take_snapshot(&SessionState, vfs_root: ContentHash) -> Result<SnapshotId, SandboxError>`: copies the tier and labels, and stores the given root hash.
  - `async restore_snapshot(SnapshotId) -> Result<Snapshot, SandboxError>`: returns the stored record, or `not_found`. It does not return a session.
  - `async fork_snapshot(SnapshotId, count: usize) -> Result<Vec<SessionState>, SandboxError>`: returns `count` new `SessionState` values that share the snapshot's `vfs_root`, with `tenant_id` set to nil.

`Snapshot` and `SnapshotId` are defined in `sandbox-types`.

## Planned design

- Snapshot is a CAS object containing the VFS root, env, cwd, tier, optional V8 heap blob, creation time, and labels.
- `take` commits the VFS, serializes shell state, and optionally captures the V8 heap.
- `restore` mounts the root hash and applies the runtime blob if present.
- `fork` gives each session its own overlay over a shared base. VM snapshots are a separate kind, planned for M5.
- Retention policies per label, with garbage collection through `sandbox-cas` mark-and-sweep.

## Invariants and security assumptions

- A snapshot's `vfs_root` is a content hash, so the same root always means the same tree.
- Forks must not share mutable overlays. Not implemented yet, because forks currently have no overlay.
- Snapshot records must be tenant-scoped before they are shared. `fork_snapshot` sets `tenant_id` to nil today, so forks are not attributed to a tenant.

## Example

Not provided. The API does not yet produce a runnable session, so an example would not show the intended use.

## Spec

See [PLAN.md](../../PLAN.md) section 4.9. Performance targets: snapshot under 5 ms, restore under 25 ms, fork under 5 ms. Nothing has been measured.
