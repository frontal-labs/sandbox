# 0047: Streaming line filters, drained input for other readers, and line editing while a command runs

## Status
Accepted. It extends ADR 0046. Pipelines are still open (see below).

## Date
2026-11-11

## Context
ADR 0046 streamed a top-level `cat` only. ADR 0046 listed the other line filters as next steps.
Two behaviours were also wrong for a terminal: a command that needs all of its input (`sort`) saw
nothing typed, and Backspace during a running command had no effect.

## Decision
- A command whose output for each input line depends only on that line streams: `cat`, `rev`,
  `tr` (with two sets), `cut` (with `-d`, `-f`, `-c`, `-s`), `sed` (one script, `-n` and `-E`),
  `grep` (with `-F`, `-i`, `-E`, `-v`), `head -n N`, and `nl` (default or `-b a`). `stream.rs`
  lists these and refuses anything else.
- A streamed command runs its own builtin on each line, so its output is the same as when it reads
  a whole buffer. `head` stops reading after its lines. `nl` keeps its own counter.
- A command that needs all of its input reads the live input to its end first. Which commands read
  standard input is decided by `reads_standard_input`: `tr`, `tee`, and `xargs` always; `cat`,
  `wc`, `sort`, `uniq`, `head`, `tail`, `rev`, `nl`, `base64`, `paste`, and `cut` when no file is
  named; `grep` and `sed` when the pattern or script is the only operand. Other commands keep their
  earlier behaviour and take no standard input.
- The PTY edits typed text as a line before the command reads it. Backspace erases, Enter sends the
  line with its newline, Ctrl-D on a partial line sends it without a newline, and Ctrl-D on an empty
  line ends the input. Ctrl-C still stops the command.

## Consequences
### Positive
- `rev`, `grep`, `sed`, `cut`, `tr`, `head`, and `nl` print each line in the PTY as it is typed.
- `sort`, `wc`, and `uniq` print once the input ends, with the typed lines in them.
- Backspace works during a running command.
- The shell tests cover streaming, no duplicated output, `head` stopping, and that `sort` waits for
  the end. The PTY tests cover typing into `cat`, Backspace, and Ctrl-D on a partial line.

### Negative
- A pipeline does not stream: in `cat | grep x` the stages still run one after the other, each on a
  whole buffer. This is the open item this change does not cover. It needs the pipeline runner to
  pass each stage's lines to the next as they arrive, and a conformance case for the pipe.
- `tail`, `sort`, `wc`, `uniq`, and `paste -s` cannot print before the input ends. This is inherent
  to what they compute.
- A command with a flag that needs a count or a line number (`grep -c`, `grep -n`) takes the
  whole-input path, so it prints only after Ctrl-D.

## References
- ADR 0046 (live stdin for `cat`), ADR 0045 (running-command input), ADR 0043 (PTY shell), ADR 0028
  (per-byte streaming), ADR 0032 (cancel).
