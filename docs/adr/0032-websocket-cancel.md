# 0032: Cancelling a WebSocket command

## Status
Accepted

## Date
2026-11-03

## Context
ADR 0030 set the WebSocket exec route at `/v1/sessions/{id}/ws`. It decided that closing the socket does not cancel the command, because the shell could not be cancelled. Cancellation then became possible, and the decision needs revisiting. The shell now yields at every command, so a run can be dropped at a clean await point. A command that keeps running after its client has gone would hold the session's gate and block other work on the session with no one left to read its output.

## Decision
- A client may send `{"type":"cancel"}` at any time while a command runs. The server stops the command and sends an `error` event with code `CANCELLED`, then a close frame.
- Closing the connection, or any read error on it, also cancels the command. The output has no reader after that. The session's gate is released when the cancelled run ends.
- A cancelled run is dropped at its next await, as a timed-out run is. Its partial writes stay in the filesystem, and the trace records the run as it does a timeout.
- The output observer is detached when the run ends by any route, including cancellation. Otherwise the event stream would never close.
- The shell yields before each command. A long loop therefore holds its runtime worker for one command at a time, and a cancel is seen within one command's work.
- NDJSON streaming (ADR 0020) still runs a command to its end when its client disconnects. Its handler does not read client messages, so it keeps ADR 0020's behaviour. This ADR changes the WebSocket route only.

## Consequences
### Positive
- A client can stop a command without closing the session, and a dropped connection no longer leaves a command holding the session.
- The session gate is released promptly after a disconnect.

### Negative
- A client that disconnects loses the rest of the command's output and its exit status. The trace still records the run, as far as it ran.
- The NDJSON and WebSocket routes now differ on disconnect. That difference is deliberate, and it is documented here.

### Risks
- A cancelled command may leave a partial write, as a kill would. The caller sees the `CANCELLED` error and can clean up.

## Alternatives Considered
- Keep the command running after a disconnect, as ADR 0030 did. Rejected: it holds the session's gate with no reader.
- Cancel on disconnect for NDJSON too. Deferred: NDJSON has no client message channel, and ADR 0020 chose completion there.

## References
- ADR 0030 (WebSocket route; its cancellation decision is superseded here), ADR 0020 (NDJSON streaming), ADR 0028 (per-byte streaming).
