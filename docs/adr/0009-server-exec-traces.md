# 0009: Server exec traces

## Status
Accepted

## Date
2026-10-15

## Context
Traces (ADR 0007) were recorded only by the local CLI. The server ran commands, but the transcript of what a session ran was not kept, so a server session could not be replayed.

## Decision
- Every session keeps a trace of the commands it ran through `POST /v1/sessions/{id}/exec`. Each command is recorded in the ADR 0007 format, as a run with the argv joined into one script. Each argument is single-quoted, so the script means the same as the argv. Replaying the script through the shell gives the same arguments, and so the same result for builtins.
- Failed commands are recorded too, as `error` runs.
- `GET /v1/sessions/{id}/trace` returns the trace as JSON Lines in the ADR 0007 format, with content type `application/x-ndjson`. It is readable by the session's owner only, like the rest of the session.
- A trace holds at most 4096 runs. Later runs are not recorded, and the response carries `X-Trace-Truncated: true`. Exec still succeeds, so a long session never fails because of its trace.
- A command that sets environment variables is not recorded, because the values are often secrets. The trace is marked incomplete instead, and no value is written.
- The trace is kept with the session. A sleeping session keeps its trace, and it is persisted with the sleeping session (ADR 0008). A destroyed session's trace is discarded.
- File writes through the filesystem API are not recorded. A trace is a list of commands, not a file history. (Amended by ADR 0012, which records filesystem writes and deletes as runs.)

## Consequences
### Positive
- A server session can be replayed with `sandbox replay` (frontal-sandbox is a legacy alias), using the same tool as local traces.
- The trace format is unchanged.

### Negative
- The argv-as-script form replays identically only for commands whose behavior depends on their arguments and on the shell state the trace captures. A command that reads a file written through the filesystem API will diverge on replay.
- The trace lives in server memory for live sessions, so a restart loses it, like the session.
