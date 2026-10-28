# 0012: Filesystem writes in traces

## Status
Accepted

## Date
2026-10-19

## Context
ADR 0011 records the files each command changed, but not the writes made through the filesystem API (`PUT` and `DELETE` on `/v1/sessions/{id}/fs/...`). Those writes happen outside any command, so no run claimed them. PLAN.md §14 lists this as the one gap left in the M2 criterion "trace shows every file change".

Replay (ADR 0007) runs scripts only. A trace that omitted these writes could not reproduce the state a later command depends on.

## Decision
- A trace run is either a script or a filesystem operation. The new `fs` field on a run holds the operation: `write` (path and the exact bytes, hex-encoded like output) or `delete` (path). A run with `fs` has an empty `script`, and a line with both is refused when the trace is read.
- The server records each `PUT` and `DELETE` as a run, with the same outcome and file list as a command (ADR 0011). A write whose data is larger than the trace's byte budget is not copied. The trace is marked incomplete instead.
- Replay re-applies each filesystem run through the same functions the server uses (`sandbox_session::write_file`, `delete_file`), so a replayed write takes the same path as the original. `sandbox_session::replay_run` runs any recorded run, script or filesystem, in a session. The `replay` function now passes the whole run to its callback, so a caller can tell the two apart.
- The write and delete functions are the ones the server handlers now call. The VFS error mapping moved to `sandbox_session::vfs_error` for the same reason.
- The format version stays 1. Old traces, which have no `fs` field, parse unchanged. A build from before this ADR ignores `fs` and would read a filesystem run as an empty script, so it must not replay a trace that has filesystem runs.
- The filesystem API does not create parent directories, so a write into a missing directory returns 404 and records nothing. That behavior is unchanged.

## Consequences
### Positive
- A trace shows every change to a session's files, whether a command made it or the API did, and replay reproduces the writes.
- Filesystem operations use the same recording path as commands, so their budget, incomplete-marking, and output handling match.

### Negative
- A trace now holds file data for every write through the API, not only for commands. The 8 MiB trace budget covers it, and a write larger than the budget makes the trace incomplete.
- The format has a second kind of run, so readers must check `fs` before they treat a run as a script.

## Related
- [ADR 0007](0007-trace-format-and-replay.md): the trace format this extends.
- [ADR 0009](0009-server-exec-traces.md): server exec traces. Its statement that filesystem writes are not recorded is amended by this ADR.
- [ADR 0011](0011-file-changes-in-traces.md): file changes in traces, which this ADR extends to filesystem runs.
