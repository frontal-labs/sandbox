# 0016: Log-structured durable store

## Status
Accepted

## Date
2026-10-19

## Context
The durable state was one file, `state.json`, rewritten whole on every change (ADR 0008, ADR 0010). Each save wrote every key, snapshot, and session with its checkpoint and trace, so the cost of one change grew with the size of the whole server. ADR 0008 and ADR 0010 both recorded this as the thing to replace.

## Decision
- **Base and log.** `state.json` is a full snapshot, as before, plus a `log_epoch` field. `state.<epoch>.log` holds JSON lines, one per changed entity: `put_key`, `del_key`, `put_snapshot`, `del_snapshot`, `put_session`, and `del_session`.
- **Saves append what changed.** Each entity's canonical JSON (object keys sorted) is hashed. A save compares each entity's hash with the one last written, and appends a put for a changed entity and a delete for a removed one. An unchanged save appends nothing. The log is synced after each append.
- **Compaction.** When the log outgrows the larger of 1 MiB and the base file, the store writes the full state as a new base file with the next epoch, creates an empty log for that epoch, and removes the old log. A crash at any step leaves a readable store, because the base names its epoch and only that epoch's log is replayed. A stale log from another epoch is ignored.
- **Torn tails.** A final line without its newline is a write a crash cut short. Open drops it and truncates the file, so the next append starts on a clean line. A damaged line anywhere else is an error, because it cannot be a torn write, and the server refuses to start rather than guess.
- **Secrets.** Environment values are still not saved (ADR 0010). A test scans every state file for a value a session set, so a change to the format that writes it fails.
- **Compatibility.** Version 2 base files without `log_epoch` load as epoch 0. The store still reads version 1 files.

## Consequences
### Positive
- A change writes one record, not the whole server. A save of an unchanged state writes nothing.
- A crash cannot lose a change that was synced, or leave the store unreadable.

### Negative
- Each save still serializes every entity to compute the hashes, so the CPU cost of a save grows with the number of sessions. Only the I/O is incremental. Keeping hashes in memory across saves avoids rereading the disk, but not the serialization.
- The log is replayed in full on open. Compaction bounds its size, not the replay time for a log just under the threshold.
- This is a single-node store. The clustered store with Postgres (PLAN §4.14 and M5) is not built.

## Related
- [ADR 0008](0008-durable-server-state.md) and [ADR 0010](0010-durable-live-sessions.md): the state this replaces the full rewrite of.
