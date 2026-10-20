# 0021: Streaming tool output as it is written

## Status
Accepted. It extends ADR 0020, whose per-command granularity still holds for every command that is not a tool.

## Date
2026-10-20

## Context
ADR 0020 sends a command's output when the command finishes. That is too late for a long `node` script or a WASI tool that prints over time. PLAN §4.15 asks for streaming, and PLAN §14 listed per-byte output as open.

Two producers can report output as it is written: the V8 print op behind `console.log` (`sandbox-runtime`) and the WASI `fd_write` host call (`sandbox-wasm`). Shell builtins return their output at the end, so they are not changed.

## Decision
- **The runtime and the WASI host take an observer.** `execute_script_streamed` and `run_module_with_mount_streamed` call the observer with each piece of stdout they keep. The observer gets exactly the bytes that go into the captured result, after the output cap is applied, so the pieces concatenate to the captured stdout.
- **Only stdout of a tool streams.** Stderr of a tool is sent when the command finishes. A tool's stdout is the only stdout its command produces, and it is written in order, so the streamed bytes are a prefix of the command's stdout.
- **The shell counts what was streamed.** A simple command that may stream gets a counter. A tool adds to it as it writes. When the command finishes, the shell sends only the stdout after that prefix, and all of its stderr.
- **A command streams only when its output is plain.** It must be at the top level (not inside a pipe, a substitution, or a redirected compound), and it must have no redirections. Other output goes through the file or pipe it was sent to, so streaming it would be wrong.
- **The output cap is the same.** Streamed bytes stop at the cap, like the captured bytes do. When the shell's own run budget is hit, the final result is truncated and its stderr is replaced, so the chunks can then run past the result. The result is authoritative in that case, and this is documented in the observer's doc comment.

## Consequences
### Positive
- `node -e` with several `console.log` calls sends each line as it is printed. A WASI tool sends each `fd_write`.
- The existing invariant holds: the chunks add up to the final result. Tests check it for node and WASI output, including errors after output.

### Negative
- The observer runs on the guest's thread, so it must not block. The server's observer sends to an unbounded channel, which does not block.
- Builtins still send at the end of the command. Per-write streaming for shell builtins would need every builtin to write through a sink, which is a separate change.

## Alternatives Considered
- **Stream every stream of every command through the shell.** Rejected. The shell's builtins build their output in buffers, and the result would depend on the order in which the shell wrote them.
- **Make the observer part of the result type.** Rejected. Callers that do not stream would pay for it, and the result would carry the same bytes twice.

## References
- [ADR 0020](0020-streaming-exec.md): per-command streaming, which this extends.
- `crates/sandbox-runtime/src/ops.rs` (`ScriptOutput::push`), `crates/sandbox-wasm/src/wasi.rs` (`fd_write`), `crates/sandbox-shell/src/interpreter.rs` (`tool_stdout`, `emit_after`).
- `crates/sandbox-shell/tests/streaming.rs`, `crates/sandbox-wasm/src/wasi.rs` (`each_stdout_write_reaches_the_observer_in_order`).
