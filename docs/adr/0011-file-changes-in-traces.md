# 0011: File changes in traces

## Status
Accepted

## Date
2026-10-19

## Context
A trace (ADR 0007) lists the commands a session ran, with their outputs. PLAN.md §4.12 and the M2 acceptance criteria require the trace to show every file change. Until now it showed none, because nothing compared the filesystem before and after a command.

Watch events (ADR 0005) are not enough. They name a path, but not what it held before or after, and a slow watcher can overflow and drop events. A trace needs exact before and after content hashes.

Committed trees are Merkle objects (PLAN.md §4.3). A directory's hash covers its whole subtree, so two committed roots can be compared without reading the files that did not change.

## Decision
- `sandbox_vfs::diff_trees(store, from, to)` walks two committed roots and returns every path that differs, sorted by path. Directories whose hashes match are skipped without being read. A missing node is an error, not an empty tree, so a damaged store cannot report a whole tree as deleted.
- Each change has an operation (`FileOp`: `create`, `write`, `delete`, `mkdir`, `symlink`), and for regular files the content hashes before and after. Directories and symlinks carry no content hash.
- A command's changes are the diff between the committed root before it runs and the committed root after it. `sandbox-session` exposes `committed_root` and `changed_since`. Both the server exec handler and `sandbox run`/`sandbox shell` use them.
- Each `RunRecord` gains an optional `files` list. The line format writes it only when it is non-empty, so existing traces parse unchanged and the format version stays 1. Replay still compares outputs and exit codes only. File state depends on the starting state, which replay does not restore.
- If a root cannot be read, the run is kept with an empty file list, and the trace is marked incomplete (`X-Trace-Complete: false`). A trace never claims a complete file list it does not have.
- Known limits, which are reported rather than hidden:
  - A rename appears as a `delete` and a `create`. A committed tree stores no file identity to tie them together.
  - A chmod is not reported. A committed tree does not store permission bits.
  - Writes through the filesystem API (`PUT /v1/sessions/{id}/fs/...`) happen outside any command. ADR 0012 records them as runs of their own, with the same file list.
  - Each command takes two checkpoints. A checkpoint clears the overlay after it commits, so it hashes only what was written since the previous checkpoint. Its other cost is reading and rewriting the entry list of each changed directory, which grows with the directory's width. A command that changes nothing still rewrites its ancestor directories, and those hash to the same blobs, so the store does not grow for them. Measured on this machine (debug build, 2000 entries in one directory, one new file per command): about 1.5 ms per command, including the exec and both checkpoints.

## Consequences
### Positive
- A trace now answers which command created, changed, or deleted which path, with content hashes that identify the bytes.
- The trace file format is backward compatible. Old readers ignore `files`, and new readers parse old traces.
- The diff is a pure function over committed trees, so it is tested without a session.

### Negative
- Every command pays for two checkpoints. Their cost is described above. Changed files add blobs to the store until GC exists (PLAN.md §14).
- The exec path's file tracking reads through the shared blob store. A checkpoint that fails makes the trace incomplete, but the command still runs.

## Related
- [ADR 0007](0007-trace-format-and-replay.md): the trace format this extends.
- [ADR 0009](0009-server-exec-traces.md): server exec traces.
- [ADR 0012](0012-filesystem-writes-in-traces.md): filesystem writes and deletes as trace runs.
- [ADR 0005](0005-vfs-watch-delivers-events-through-a-stream.md): watch events, which are not used for traces.
