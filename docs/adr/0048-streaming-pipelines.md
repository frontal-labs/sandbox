# 0048: Streaming pipelines over live input

## Status
Accepted. It completes the streaming work of ADRs 0046 and 0047.

## Date
2026-11-11

## Context
ADR 0047 left pipelines open: `cat | grep x` ran each stage on a whole buffer, so nothing printed
until the input ended. A pipeline whose first stage needed all of its input got none of the typed
text.

## Decision
- A pipeline over live input streams when it has at least two stages, every stage is a builtin
  with plain words (no expansion, no glob characters) and no redirection, and every stage but the
  last is line-local (see `stream.rs`). The last stage may need all of its input.
- Each complete line passes through the stages in order. A stage that produces nothing for a line
  stops that line. The last stage's output is reported as it is produced when it is line-local. A
  last stage that needs all its input runs on the chain's whole output when the input ends.
- `head` at any stage ends the run, as it does alone, because the chain has no more input to use.
- A pipeline whose first stage needs all of its input drains the live input to its end, then runs
  as before. This is what `sort | rev` does in the PTY.
- Pipelines that are not plain (expansions, redirections, functions, external commands, assignments)
  keep the earlier behaviour.

## Consequences
### Positive
- `cat | rev`, `grep a | rev`, and `cat | head -n 1` print as lines arrive in the PTY.
- `cat | sort` and `sort | rev` print once the input ends, with what was typed.

### Negative
- The exit status of a streamed pipeline is 0. The earlier pipeline runner reported the last stage's
  status. This needs the per-line status carried through the chain, which is not done yet.
- A pipeline using a word with expansion (`cat | grep $PATTERN`) or a redirection falls back to the
  whole-buffer runner, so it still prints only when the input ends.
- External commands in a pipeline do not stream.

## References
- ADR 0047 (streaming line filters), ADR 0046 (live `cat`), ADR 0045 (running-command input).
