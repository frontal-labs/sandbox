# 0037: Signals on the WebSocket exec route

## Status
Accepted

## Date
2026-11-10

## Context
PLAN §4.15 describes the interactive WebSocket as "exec, stdin, signals". ADR 0032 added cancellation with `{"type":"cancel"}`. Signals were still missing. Stdin beyond the request's `stdin` field is a separate change: the shell takes its input as a fixed byte slice, and making it a live stream touches the interpreter, the builtins, and the runtimes. That is not decided here.

## Decision
- A client may send `{"type":"signal","name":"<NAME>"}` while a command runs.
- The stopping signals are `INT`, `TERM`, `KILL`, and `HUP`. Each one stops the command exactly as a cancel does: the server sends an `error` event with code `CANCELLED`, then a close frame.
- Any other signal name is ignored, and the command keeps running. Messages with an unknown `type` are ignored too, as they were before.
- The shell has no processes or PIDs (PLAN §4.4), so the four names all map to one stop. They are accepted so that a client written against a real Unix shell can send the names it already uses.

## Consequences
### Positive
- The signal part of §4.15 is served, and it reuses the cancel path from ADR 0032, so the trace, the session gate, and cleanup behave the same.

### Negative
- `INT`, `TERM`, `KILL`, and `HUP` do not differ in behaviour. A caller that expects `TERM` to let a command clean up will be disappointed. The shell has no handlers to run, so nothing can differ.
- A signal the server does not stop on gets no reply. The client learns that nothing happened from the output alone.

### Risks
- Stdin is still not live. A client that needs to write after the request must wait for a later change.

## Alternatives Considered
- Reject unknown signal names with an error event. Not chosen: the watcher task has no channel to send events on, and the forwarder owns the sink.
- Deliver `SIGSTOP`/`SIGCONT`. Not chosen: the shell cannot pause a command at a defined point.

## References
- PLAN §4.15, §4.4. ADR 0030 (WebSocket route), ADR 0032 (cancellation).
