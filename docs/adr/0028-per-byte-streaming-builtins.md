# 0028: Per-byte streaming for shell builtins

## Status
Accepted

## Date
2026-11-01

## Context
ADR 0020 added per-command streaming: a command's output is sent when it finishes. ADR 0021 extended it for tools: the V8 runtime and WASI host report stdout as it is written, so a long `node` script or WASI tool streams each `console.log` or `fd_write`. But shell builtins still buffer their output and send it at the end of the command. PLAN §4.15 and §14 leave per-byte streaming for builtins open.

Builtins that produce a lot of output (like `cat`, `head`, `tail`, `grep`, `ls`) would benefit from streaming: a `cat` of a large file could send each chunk as it reads it, rather than buffering the whole file in memory.

## Decision
- **Builtins write through a sink.** A new `BuiltinStdout` type, similar to `ToolStdout`, is passed to builtins. Builtins write to it incrementally, and it reports to the run's `OutputObserver` as bytes are written.
- **Only when output is plain.** A builtin streams only when its output reaches the top level: no pipe, no redirection, no substitution, no capture. This matches the tool streaming rule.
- **Stderr still buffers.** Only stdout streams; stderr is sent when the command finishes, as with tools.
- **The observer gets exactly the bytes that go into the captured result.** Streamed pieces concatenate to the captured stdout, after the output cap is applied.
- **Builtins are updated one by one.** The change is mechanical: replace `ok(text.into_bytes())` with writes to the sink. The signature of `builtin` changes to accept the sink.

## Consequences

### Positive
- `cat bigfile` streams each read, instead of buffering the whole file.
- `grep` streams matches as they are found.
- `ls` on a large directory streams entries as they are read.

### Negative
- Every builtin must be updated. This is a mechanical but widespread change.
- The output cap applies to streamed bytes, so a very long stream from a builtin can be truncated.

## Alternatives Considered
- **Stream at the interpreter level.** Rejected. The interpreter doesn't know when a builtin's stdout is produced; the builtin does.
- **Stream stderr too.** Rejected. Tools don't stream stderr, and keeping parity simplifies the model.
- **Don't do it.** Rejected. The PLAN lists it as a milestone, and it enables memory-efficient processing of large outputs.

## References
- [ADR 0020](0020-streaming-exec.md): per-command streaming.
- [ADR 0021](0021-streaming-tool-output.md): tool streaming, which this mirrors for builtins.
- `crates/sandbox-shell/src/interpreter.rs`: `OutputObserver`, `tool_stdout`, `emit_after`.
- `crates/sandbox-shell/src/builtins.rs`: all builtins.