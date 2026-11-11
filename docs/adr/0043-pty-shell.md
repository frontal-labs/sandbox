# 0043: Interactive PTY shell over WebSocket

## Status
Accepted

## Date
2026-11-11

## Context
PLAN §6 (M6) asks for an interactive PTY shell. A sandbox session runs commands in-process (the
isolate tier) with no processes or PIDs (PLAN §4.4), so a terminal has to be emulated on the
server. The WebSocket route already carries a command with cancel and stopping signals (ADR 0032,
0037), and sessions keep their working directory and environment between `run_script` calls.

## Decision
- `GET /v1/sessions/{id}/pty` upgrades to a WebSocket and serves an interactive shell on the
  session. Messages are JSON text frames:
  - client → server: `{"type":"input","data":"..."}` for typed text, and
    `{"type":"resize","cols":N,"rows":N}` for the terminal size.
  - server → client: `{"type":"output","data":"..."}` and `{"type":"exit","code":N}`, then a close.
- The server edits the typed line as a line discipline does: echoes characters, handles Backspace
  and Delete, runs the line on Enter (CR or LF), discards it on Ctrl-C, and ends the shell on Ctrl-D
  of an empty line. Other control characters are ignored.
- `exit` and `exit N` end the shell with status N (0 when no number is given). The shell does not
  carry the last command's status here, so a bare `exit` is 0.
- A command runs in the session through `run_script`, under the session's gate, as an exec does.
  Ctrl-C while it runs drops it at its next await point (ADR 0032), echoes `^C`, and prints a new
  prompt. Other input typed while a command runs is discarded.
- Resize sets `COLUMNS` and `LINES` in the session, so commands can read the size.
- Output is written with CR LF line ends, as a terminal expects.

## Consequences
### Positive
- A client gets a shell with echo, editing, interrupts, and an exit status, over one connection.
- The line editor is a pure state machine with unit tests; the WebSocket driver has integration
  tests for session start, typed input and output, resize, Ctrl-C on a running command, `exit N`,
  and Ctrl-D.

### Negative
- Typed input is not an interactive stream into a running command. A command reads no stdin from
  the terminal, because the shell takes each command's input as a whole buffer (ADR 0038).
- Commands run from the PTY are not checkpointed when they finish. A durable server records them
  at the next exec, snapshot, or sleep, so a restart before then loses their filesystem changes.
  Closing this gap needs the checkpoint call the exec path makes, passed into the driver. It is
  the open item of this change.
- A bare `exit` reports 0, not the last command's status.

## Alternatives Considered
- A real pseudo-terminal from the host. Not chosen: the isolate tier has no host processes to
  attach one to, and PLAN §4.4 puts the shell in-process.
- Reusing the exec WebSocket with a mode flag. Not chosen: the PTY needs input while a command
  runs, which the exec protocol does not describe, so a separate route keeps both protocols clear.

## References
- PLAN §4.4, §4.15, §6 (M6). ADR 0030 (WebSocket route), ADR 0032 (cancel), ADR 0037 (signals),
  ADR 0038 (live stdin).
