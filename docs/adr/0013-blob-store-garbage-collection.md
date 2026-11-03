# 0013: Blob store garbage collection

## Status
Accepted

## Date
2026-10-19

## Context
The blob store only grows. Every checkpoint, snapshot, sleep, and command writes the committed tree, and nothing removes a blob once no session or snapshot names it (PLAN.md §4.2 and §14). Per-command file tracking (ADR 0011) makes this faster. PLAN.md §4.2 requires garbage collection by mark and sweep from live roots, and requires that it never run inline on the request path.

Two properties make a sweep dangerous. A blob written during a sweep can be swept before anything refers to it, and a blob shared by two trees can be removed while one still needs it. The first comes from concurrent writes. The second comes from chunk files, which identical content shares.

## Decision
- **Roots.** A collection marks from three kinds of root: the committed root of every live session, the checkpoint each durable session keeps, and every snapshot record. A sleeping session has no live session, but sleep records a snapshot of its root, so it is covered.
- **Mark.** `sandbox_vfs::reachable_blobs` walks the committed trees from the roots. It records each node object and each file's contents. A node that cannot be read is an error, so a damaged tree is never treated as empty.
- **Sweep.** Every blob listed at the start of the collection that is not marked is deleted. Blobs written after the listing are not in the list, so they survive. Chunk files are then removed when no live blob's manifest names them. A chunk can be shared, so `delete` never removes chunks; only `delete_parts`, given the live set, does.
- **Locking.** The collector takes the gate of every session, as commands and filesystem operations do, and then the session table's write lock. It checks that the table has the same sessions it gated, and starts again if not. While it holds both, no session can write, and no session can be created or destroyed. The gates are taken one at a time without the table lock, so this cannot deadlock with a command, which holds a gate and then the table.
- **Trigger.** `frontal-sandbox serve` runs a collection every `--gc-interval-secs` seconds (default 600, 0 turns it off). The first one runs after one interval. `POST /v1/gc` also starts a collection in a background task and returns `202 Accepted`. The report goes to the log. One collection may start per minute, and a call inside that window returns `429 RESOURCE_EXHAUSTED`. Every collection holds every session's gate for a moment, so an unlimited trigger would let one key stall the server.
- **Store support.** The blob store trait gains `list_parts`, `parts`, and `delete_parts`, with defaults for stores that do not chunk. `DiskStore` implements them. `list` now reports chunked blobs too. Before this change it skipped them, because a chunked blob has a manifest and no file.

## Consequences
### Positive
- Storage held by unreachable trees and chunks is reclaimed.
- The collector cannot remove a blob a session or snapshot can still reach, and it cannot race a write.

### Negative
- Every collection waits for all sessions' gates. A long command on any session delays it.
- The schedule is set on the `serve` command line, not in the server library. A library caller that wants a schedule calls `sandbox_server::gc::schedule` with the state from `create_router_with_state`.
- A session created or destroyed during a collection makes it start again. Under heavy churn it can run several times before it completes.

## Related
- [ADR 0004](0004-snapshot-records-live-in-the-server.md): snapshot records, which are roots.
- [ADR 0006](0006-sleep-and-wake.md): sleep, which records a snapshot.
- [ADR 0010](0010-durable-live-sessions.md): durable checkpoints, which are roots.
- [ADR 0011](0011-file-changes-in-traces.md): per-command checkpoints, which add blobs.
