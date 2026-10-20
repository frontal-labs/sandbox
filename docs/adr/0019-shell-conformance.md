# 0019: Shell conformance against GNU bash

## Status
Accepted

## Date
2026-10-20

## Context
PLAN.md §7 requires a conformance suite that runs shell scripts against both the sandbox and GNU bash, and §9 M1 requires 200 or more cases to pass. Before this change the repository had no corpus, and a measured run of a first corpus of 219 cases passed 146 of them. The failures were missing builtins (`printf`, `read`, `exit`, `type`, `grep`, `tail`), parameter expansion (`${x:-y}`, `${#x}`, and the `%` and `#` forms), `case`, arithmetic, field splitting on unquoted expansions, file-descriptor redirections, and several small differences in status and output.

The expected output has to come from bash itself. A corpus whose expectations were written by hand would test the author's idea of bash, not bash.

## Decision
- **The corpus is recorded, not written.** `crates/sandbox-shell/conformance/cases.py` lists the scripts. `record.py` runs each one under GNU bash 5.3 in an empty directory and writes `cases.json`, which records the bash version. The Rust test `tests/conformance.rs` replays every case on a fresh in-memory filesystem and reports every mismatch. Scripts use only relative paths and never print the working directory, because the sandbox starts at `/` and bash starts in a host directory.
- **Compared: stdout and the exit status.** Stderr is not compared, because the wording of error messages is not part of the contract. Error messages are still checked where a test asserts them.
- **A syntax error matches status 2 with no output.** The sandbox reports a syntax error as an error from `run`, not as an exit status. Bash reports it with status 2 and no output. The harness accepts this pairing only for `ShellError`, the syntax error code, so runtime errors still fail.
- **Unquoted expansions are split on `IFS`.** This changes an earlier decision, which did not split. Output of an unquoted expansion is split into fields on space, tab, and newline. Quoted output is not split. Globbing still does not apply to expanded values, which is stricter than bash and is kept on purpose.
- **Every parameter reference is a parsed piece.** `$name`, `$1`, `$@`, `$?`, and `${...}` all become `Piece::Param`. Earlier, plain names were expanded at the character level, which could not represent `"$@"` or `${x:-y}`. The character-level expander is gone.
- **`"$@"` gives one field per parameter.** `"$@"` with no parameters gives no field, and `"$*"` gives one.
- **Redirections are applied in order, to descriptors 1 and 2.** `2>&1`, `>&2`, `2>file`, and `/dev/null` work. Descriptors above 2 are a parse error, not a silent no-op.
- **Compound commands take redirections and input.** `for ...; done > file` redirects the whole loop. A compound command reads the stdin of its pipeline stage. A simple command with no stdin of its own reads it only if it is a tool that reads stdin, such as `cat` or `grep`. `echo` leaves it alone, so `while read` loops see every line.
- **Failed expansions end the run.** An arithmetic error stops the run with status 1, as bash does.
- **Refusals stay refusals.** `${x:?msg}` and file descriptors above 2 are parse errors. They are outside the plan's scope, and an explicit error is better than a guess.

### Dependencies
`regex` is used by `grep`. It was already in `Cargo.lock` through other crates, so no new code enters the build. The `regex` crate's syntax is close to POSIX extended, not to basic regular expressions. That difference is documented in the `grep` builtin.

## Consequences
### Positive
- The corpus of 219 cases passes against bash. Each regression of a shell behaviour the corpus covers now fails a test.
- Found by running the corpus: a glob matcher that was exponential (fixed earlier), a `$(...)` in a loop body that drained the loop's input, `mv` that did not replace files, and arithmetic that refused `$` names.

### Negative
- `case`, arithmetic, and indirect expansion are now part of the shell, and each adds parser and evaluator code.
- The shell is still not bash. Anything outside the corpus and the feature tests may differ. The corpus is the measure, and it covers the plan's list, not every bash feature.

### Risks
- Recorded expectations depend on the bash version. `cases.json` records 5.3.8, and a different bash may differ on edge cases. Re-recording is the fix, and the test shows the change.

## Alternatives Considered
- **Hand-written expected output.** Rejected. It would have encoded the author's model of bash.
- **Run the corpus against a real bash inside the test.** Rejected. It would make the test depend on the host and on bash being installed. A recorded file is reproducible.
- **Keep the strict no-splitting rule and drop the corpus cases that need it.** Rejected. Field splitting is how `for w in $x` works in every script an agent writes.

## References
- [PLAN.md §4.4](../../PLAN.md): the shell's syntax and builtins.
- [PLAN.md §7](../../PLAN.md): the conformance suite.
- [ADR 0018](0018-shell-recursion-limit.md): recursion limits, which the new control flow respects.
- `crates/sandbox-shell/conformance/`: the corpus, its recorder, and `cases.json`.
- `crates/sandbox-shell/tests/features.rs`: behaviour the corpus does not isolate.
