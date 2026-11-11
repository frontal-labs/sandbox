# 0046: Live standard input for a top-level cat

## Status
Accepted. It delivers live input for `cat` only. Other readers still take their input as one
buffer, as ADR 0045 describes.

## Date
2026-11-11

## Context
ADR 0045 left typed input for a running command open. The shell took each command's input as one
buffer, so a running command could not receive keystrokes. Streaming every builtin would change
about 80 call sites at once, and it would need GNU conformance cases for each streaming form.

## Decision
- `sandbox-shell` has a `LiveInput` (`live.rs`), a channel of chunks with an explicit end. A caller
  keeps the `LiveSender` and writes what arrives.
- `Shell::run_streamed_live` gives the run a `LiveInput`. A top-level `cat` with no operands, which
  is not captured by a pipe, a substitution, or a redirection, reads it chunk by chunk. Each chunk
  is reported to the observer as it arrives, and counted in the same `stream_count` the streaming
  of builtins uses, so the command's end reports only what is left.
- The session trait has `run_script_live`. The isolate backend implements it. Other tiers refuse
  with `Unsupported`.
- The PTY runs each line with a `LiveInput` and an observer. Typed text goes into the input and is
  echoed. Output is sent as the observer reports it, and at the end only the bytes the stream did
  not already send.
- Ctrl-D during a line ends the input, so `cat` finishes. Ctrl-C still stops the line.

## Consequences
### Positive
- `cat` in the PTY prints each line as it is typed, and Ctrl-D ends it. The tests cover this
  against the shell and over the PTY socket.
- No command that reads a file or a whole buffer changes, and the conformance corpora still pass.

### Negative
- Only `cat` with no operands streams input. `grep`, `sort`, `wc`, `head`, and the other readers
  still wait for the whole input. In the PTY they see nothing typed while they run, and what is
  typed is echoed but discarded.
- A pipeline does not stream: `cat | wc` reads its input as one buffer.
- Typed text is echoed without the line editor, so Backspace during a running command is not
  applied to the input.

## Next steps
Stream the line-oriented filters the same way (`grep`, `head`, `tail`, `tr`, `cut`, `sed`, `nl`,
`rev`), each with GNU golden cases for the streaming form. `sort`, `wc`, `uniq`, and `paste -s` need
their whole input and stay as they are.

## References
- ADR 0045 (exit status and running input), ADR 0038 (live stdin on the WebSocket route), ADR 0028
  (per-byte streaming), ADR 0043 (PTY shell).
