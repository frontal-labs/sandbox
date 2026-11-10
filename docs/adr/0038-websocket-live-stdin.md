# 0038: Live stdin on the WebSocket exec route

## Status
Accepted

## Date
2026-11-10

## Context
PLAN §4.15 lists the interactive WebSocket as "exec, stdin, signals". ADR 0037 served signals. Stdin still came only from the request's `stdin` field, a fixed byte array set before the command started. A client with a large input, or one that produces input as it goes, had to fit all of it into the first message.

The shell takes each command's input as a whole buffer. Every builtin and pipeline stage reads its input to the end before it works. Streaming input into a running builtin would change about 80 call sites in `sandbox-shell`, and the request schema is shared with the OpenAPI spec and the SDKs. That is a larger change than this increment, so it is deferred (see Alternatives).

## Decision
- The route accepts an opt-in query parameter: `GET /v1/sessions/{id}/ws?stdin=live`. Without it, the route behaves as before (ADR 0030, 0032, 0037). `ExecRequest` is unchanged, so the OpenAPI spec and the SDKs do not change.
- In live mode the client may send, after its request, these messages:
  - `{"type":"stdin","data":"<text>"}` appends the text, as UTF-8 bytes, to the command's input.
  - `{"type":"stdin_eof"}` ends the input.
- A command whose name may read its input (anything that is not a non-reading builtin, and `sh` always) waits for `stdin_eof` before it starts. Its input is the request's `stdin` field followed by the chunks, in order. Waiting holds the session's gate, as a running command does.
- A command that cannot read its input (for example `echo`) starts at once. Any `stdin` messages it receives are dropped.
- While waiting, `{"type":"cancel"}` or a stopping signal (ADR 0037) ends the request with a `CANCELLED` error and a close frame, without running the command. Closing the socket while waiting also ends it, and nothing runs.
- Chunks are text. Binary input stays in the request's `stdin` field, which is a byte array.

## Consequences
### Positive
- A client can send input of any size in pieces, with no change to the request schema or to the SDKs' request types.
- The shell is unchanged. Builtins and runtimes still see one buffer.
- Existing clients are not affected, because live mode is opt-in.

### Negative
- A reading command does not run until the input is complete. Output does not begin while input is still arriving. This is not interactive stdin in the sense of a Unix pipe, and clients that need that must wait for it.
- A non-reading command ignores later chunks, so a client cannot tell from the output that its input was dropped. It is documented here.

### Risks
- A client that opens live mode, runs a reading command, and never sends `stdin_eof` holds the session's gate until it disconnects. The disconnect ends the wait, so the session is freed.

## Alternatives Considered
- Stream stdin into a running builtin. Deferred: it changes every builtin and the runtimes, and it needs its own conformance work.
- Add an `stdin_live` field to `ExecRequest`. Not chosen: it changes the request schema for every caller and the OpenAPI spec, for a feature only the WebSocket uses.
- Treat every command as a reader. Not chosen: `echo` would then wait for input it never reads.

## References
- PLAN §4.15, §4.4. ADR 0030 (WebSocket route), ADR 0032 (cancel), ADR 0037 (signals).
