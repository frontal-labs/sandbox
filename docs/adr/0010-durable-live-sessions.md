# 0010: Durable live sessions

## Status
Accepted

## Date
2026-10-16

## Context
ADR 0008 kept keys, snapshots, and sleeping sessions across a restart, but a running session was lost. ADR 0006 also dropped the working directory and environment on sleep. PLAN.md lists durable state as part of the plan, so a session should come back after a restart as it was.

## Decision
- A session's checkpoint is its committed filesystem root, its working directory, and its environment. `Session::checkpoint()` returns one and replaces `Session::sleep()`. `restore_checkpoint` starts a session from one. `snapshot()` and `restore(root)` are unchanged, and snapshots still record only the root (ADR 0004).
- On a durable server (ADR 0008), every session keeps its last checkpoint. The checkpoint is refreshed after each change to the session: `exec`, a file write, and a file delete. Creating, forking, sleeping, and waking refresh it too. Each refresh is followed by a save of the state file.
- The state file (version 2) holds every session with its checkpoint, owner, state, and trace, and a `sleeping` flag. On a restart, live sessions come back running from their checkpoint, under the same IDs. Sleeping sessions wait for a wake. Version 1 files still load, and their sleeping sessions come back with the default shell state.
- Sleep keeps the working directory and environment as well as the files. Wake restores all three.
- A change that is still running when the process dies is lost. Only a change that finished, and was saved, comes back.

## Consequences
### Positive
- A restart does not lose a running session, its files, its shell state, or its trace.
- Sleep and wake keep the shell state, which ADR 0006 did not.

### Negative
- Every change commits the filesystem and rewrites the whole state file. That is correct but does not scale to many sessions. A log-structured store would replace the file.
- Old checkpoints keep their blobs in the disk store. Nothing collects them yet, so the store grows. Garbage collection is planned under M5.
- A memory-only server skips all of this, so its sessions are still lost on restart.
