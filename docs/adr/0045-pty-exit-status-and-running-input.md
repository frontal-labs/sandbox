# 0045: PTY exit status, and typed input for running commands

## Status
Accepted. The exit status is done. Typed input for a running command stays open, for the reason below.

## Date
2026-11-11

## Context
ADR 0043 left two items open: a bare `exit` reported 0, and typed input could not reach a command
that was running.

## Decision
- A bare `exit` ends the shell with the status of the line that ran before it. A line that fails
  reports 1, and a line stopped by Ctrl-C reports 130, the usual signal convention. `exit N` is
  unchanged.
- Typed input for a running command is not implemented. The shell takes each command's input as
  one buffer that is complete before the command starts (ADR 0038), and every builtin reads it
  that way. A running command therefore has no channel from the terminal, so the PTY can only
  discard typed text while a command runs, except Ctrl-C, which stops it.

## What typed input for a running command would need
- A stdin source the interpreter reads incrementally, instead of a `Vec<u8>`. The builtins that
  read input (`cat`, `head`, `tail`, `wc`, `grep`, `sort`, `uniq`, `cut`, `tr`, `tee`, `paste`,
  `rev`, `sed`, `nl`, `base64`, `xargs`, `node`, `wasm`) would consume it line by line, and each
  would need a streaming form.
- A way for the PTY driver to write typed text into that source while the command runs, and to
  close it on Ctrl-D.
- Conformance cases against GNU for the streaming forms, because a line-at-a-time reader can
  differ from a whole-buffer one at the boundaries.
This is the same change ADR 0038 deferred. It is a shell rework of its own, and this PR does not
attempt it.

## Consequences
### Positive
- A bare `exit` now reports what the last command reported, as a shell does.

### Negative
- Commands cannot be answered interactively. A script that prompts for input (`read`, a password
  prompt) gets nothing from the terminal while it runs, and the prompt is discarded.

## References
- ADR 0043 (PTY shell), ADR 0038 (live stdin on the WebSocket route), ADR 0032 (cancel).
