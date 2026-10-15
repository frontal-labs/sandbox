# 0008: Durable server state

## Status
Accepted

## Date
2026-10-15

## Context
The server kept every key, snapshot record, and sleeping session in memory, so a restart lost them. A sleeping session is only worth having if its root survives a restart, and a snapshot is only useful if its owner can still authenticate. The content store had a disk implementation (`DiskStore`), but the server did not use it.

## Decision
- `sandbox serve --data-dir <DIR>` makes the server durable. Without the flag, it behaves as before, entirely in memory.
- Blobs go to `<DIR>/blobs` through `DiskStore`. The session backend's blob store is process-wide, so `sandbox_session::install_blob_store` installs the disk store before the first session. It fails if a store is already in use.
- `<DIR>/state.json` holds the server's durable state, and it is rewritten atomically (temp file, then rename) after every change to it:
  - API keys as `(key id, BLAKE3 digest of the secret)`. The secret is never written.
  - Snapshot records: ID, owner key ID, source session, root, creation time, labels.
  - Sleeping sessions: the session state, the sleep root, and the trace (see ADR 0009).
- On start, the keys, snapshots, and sleeping sessions are loaded. A sleeping session wakes from its stored root under its old ID. Its owner's key still works.
- The first start of a durable server creates one key and prints it. Later starts print nothing, because the key from the first start is still valid.
- Live sessions are not persisted. A running session is lost on restart, and its owner gets `not_found`. A session that should survive a restart must sleep first.
- Trust: `state.json` contains digests, not secrets, and it holds no content beyond the snapshot roots. The server creates the directory with mode 0700 and the state file with mode 0600, so only the server's user can read them. This is Unix-only; other platforms get the default permissions.

## Consequences
### Positive
- Keys, snapshots, and sleeping sessions survive a restart.
- Durability reuses the existing disk store, and the in-memory path is unchanged.

### Negative
- The state file is rewritten in full on each change. That is simple and correct, but it does not scale to many thousands of snapshots. A log-structured store would replace it.
- A process-wide blob store is installed once, so a process cannot switch data directories after it has created a session.
- Live sessions and their warm shell state are not durable.
