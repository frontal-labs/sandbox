# 0020: Streaming exec, per finished command

## Status
Accepted

## Date
2026-10-20

## Context
PLAN.md §4.15 lists `POST /v1/sessions/{id}/exec` as "streaming NDJSON or WebSocket", and §4.16 gives the SDK an `exec_stream`. Until now the server refused `stream=true` with `UNSUPPORTED`, because the shell buffered each command's output and returned it at the end. PLAN §14 left the granularity open: per command, or per byte. Per byte means every builtin, every WASI tool, and the V8 runtime writing to a sink as it goes. That is a change across four crates, and it is not needed for the first useful version.

The shell already has a single point where a command's output becomes final: `run_simple` routes a finished command's output to its sinks. Output that reaches the top level of a run is complete at that point, and it is the same bytes the final result holds.

## Decision
- **Granularity: a command's output is sent when the command finishes.** Each simple command that writes to the top level sends one chunk per stream, when it returns. A loop sends one chunk per iteration body command. A function call sends one chunk, when the call returns, because its body's output is part of the call. A pipeline sends the output of its last stage only, when that stage finishes, because earlier stages feed the next stage and never reach the top level.
- **Not streamed:** output inside a pipe, a redirection, or a substitution. The shell consumes that output, so it is not sent. Output redirected to a file or `/dev/null` is not sent either, and a redirected compound command sends its routed result once, when it finishes.
- **Exactly the same bytes.** The chunks for a run add up to the run's final stdout and stderr. The final result is still returned, so the trace, metering, and buffered callers are unchanged. A test checks the concatenation on a set of scripts.
- **Per-byte streaming was not part of this decision.** It is in ADR 0021 for WASI tools and `node`. Shell builtins still send at the end of their command.
- **Wire format: NDJSON.** One JSON object per line: `{"type":"output","stream":"stdout","data":[...]}`, then exactly one `{"type":"exit",...}` or `{"type":"error","code":...,"message":...}`. Bytes are integer arrays, as in `ExecResult`. `ExecEvent` lives in `sandbox-types`, so the server and the SDK share one type.
- **Status codes.** Failures before the response starts are HTTP errors, such as an unknown session (404) or an empty argv (400). Once output has started, the status is already 200, so a timeout or other failure is an `error` line. Clients read the last line.
- **Timeouts and disconnects.** The timeout applies to a streamed command as it does to a buffered one. A timed-out command releases its observer, so the stream ends. If the client disconnects, the command still runs to its end, and the session is saved and traced, so nothing is left half-recorded.
- **The shell's own messages.** A few messages the shell writes outside a command, such as an expansion error in a `case` subject, reach the final result but may not be sent as a chunk. They are rare, and the final result is authoritative.

## Consequences
### Positive
- A script with several commands reports each one as it finishes. The `exec_stream` in the Rust SDK reads these as they arrive.
- The design touches one place in the interpreter, so the existing 219-case conformance corpus and the feature tests are unchanged in behavior.

### Negative
- One long command, such as a single `node` run that prints for a minute, sends nothing until it ends. This is the main limit, and it is the reason per-byte streaming is still open.
- The chunk boundaries follow command boundaries, not write boundaries, so a client cannot use them to detect a particular byte early.

## Alternatives Considered
- **Per byte, now.** Rejected for this change. It needs changes in every tool and the runtime, and each one needs its own tests. It is a larger design with its own ADR when it is built.
- **Per top-level item of a script.** Rejected. `exec` takes one argv, so it would send nothing early, and loops would still wait for the whole loop.
- **WebSocket.** Not chosen. NDJSON over a plain response works with HTTP proxies and with `curl`, needs no new protocol state, and matches the trace endpoint's existing `application/x-ndjson`.

## References
- [PLAN.md §4.15, §4.16](../../PLAN.md): the endpoint and the SDK method.
- [ADR 0019](0019-shell-conformance.md): the shell behavior the streaming must not change.
- `crates/sandbox-shell/src/interpreter.rs`: `emit`, `OutputObserver`, and the capture rules.
- `crates/sandbox-shell/tests/streaming.rs`, `crates/sandbox-server/tests/rest.rs`, `sdks/rust/tests/against_server.rs`.
