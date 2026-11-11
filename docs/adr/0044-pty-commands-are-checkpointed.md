# 0044: PTY commands are checkpointed, traced, and metered

## Status
Accepted

## Date
2026-11-11

## Context
ADR 0043 left one gap open: a command run from the PTY shell was not checkpointed when it
finished, and it was not written to the trace or the meter. A durable server would have lost its
filesystem changes after a restart, unless another exec or snapshot happened first.

## Decision
- The PTY driver no longer holds a session. It calls a `Runner` (in `pty.rs`), which the server
  implements (`PtyRunner` in `api.rs`).
- `run` executes a line under the session's gate, as an exec does, and records the committed root
  and generator state before it starts.
- `finish` runs after every line, whether it finished, failed, or was stopped by Ctrl-C. For a line
  that ran, it records the trace entry and the meter reading. It always checkpoints the session and
  saves it, so the writes a stopped line made before it stopped are kept too.
- If `finish` fails, the shell reports the error and ends with status 1, so a line whose state
  could not be saved is not silently accepted.
- The trace records each line as `sh -c <line>`, so replay runs the same script.

## Consequences
### Positive
- A file written from the PTY survives a restart on a durable server. The test
  `pty_durable.rs` checks this, and disabling the bookkeeping makes it fail with 404.
- PTY commands appear in the trace and the meter, like exec commands.

### Negative
- Each line costs a checkpoint and a save, as an exec does. A busy interactive session writes
  more often than a batch of execs would.
- A line stopped by Ctrl-C is checkpointed without a trace entry or meter reading, because it has
  no finished result. Its partial writes are kept, as ADR 0032 describes.

## References
- ADR 0043 (PTY shell, where the gap was recorded), ADR 0032 (cancel), ADR 0023 (state saves).
