# 0014: Session limits

## Status
Accepted

## Date
2026-10-19

## Context
The server refused every custom resource limit with `501` (`refuse("per-session resource limits")`). The VFS already enforces four of them as quotas: file size, total bytes, inode count, and path depth. The backend ignored `SessionConfig` entirely, so those quotas were the defaults for every session, whatever the caller asked for.

A quota that is set on create but dropped by a wake, a fork, or a restart would be worse than a refusal. The caller would believe a limit holds that does not. PLAN §4.7 requires resource limits that every backend shares.

## Decision
- **Enforced:** `max_file_size`, `max_total_size`, `max_inodes`, and `max_path_depth`. They map to the VFS `Quotas` (`sandbox_session::quotas_for`). A field left unset takes its default. A zero value is refused as `INVALID_ARGUMENT`, because it would block every write.
- **Still refused with `501`:** `memory_bytes` and `cpu_ms_per_sec`, which have no enforcement; `max_processes` and `max_open_files`, which the shell does not count; and a `wall_timeout_secs` other than the default, because the command cap applies only at the default (ADR 0013 and LIMITS.md).
- **Kept with the session.** The limits are stored on the session entry, copied into each snapshot record, and saved with the session and snapshot state (`limits`, with a serde default, so state written before this change loads with the defaults). A sleep, a wake, a restart, and a fork each restore the limits they had.
- **Persisted sessions.** A session is now saved when it is created and when it is forked. Before this, a session that was created and then survived a restart without a write was lost. This was a durability bug (ADR 0010), and this ADR fixes it along with the limits.

## Consequences
### Positive
- A quota a caller sets holds for the life of the session, through sleep, restart, and fork.
- The four enforced limits are real limits, not documented intentions.

### Negative
- Quotas are checked when a write is made, not reserved, so concurrent writers can overshoot. LIMITS.md already records this.
- A fork of a snapshot takes the limits of the session the snapshot came from, not the limits of the caller who forks it. That matches the snapshot, which is the state being forked.
- Memory and CPU limits are still unenforced, so a session can use as much memory and CPU as the process has.

## Related
- [ADR 0006](0006-sleep-and-wake.md), [ADR 0008](0008-durable-server-state.md), [ADR 0010](0010-durable-live-sessions.md): the paths that now keep limits.
- [ADR 0013](0013-blob-store-garbage-collection.md): the command wall timeout that remains the only time limit.
