# 0018: Shell recursion limit and abort propagation

## Status
Accepted

## Date
2026-10-19

## Context
The isolate backend runs the shell inside the server process (PLAN.md §5, and `docs/SECURITY_MODEL.md`). A stack overflow aborts the whole process, and it cannot be caught. Two paths could overflow the stack:

- **Parser.** `$(...)` nesting was capped at 32 levels, but `if`, `while`, `for`, `{ }`, and `( )` nested without limit, and each level recursed in the parser.
- **Interpreter.** Nothing bounded the depth of runtime recursion. A function whose body calls itself (`H() H`) recursed until the step budget ran out, which was 1,000,000 steps, and the stack overflowed long before that. The fuzzer `fuzz/shell_run` found it with a 10-byte input.

A limit that stops the run also has to fail the run. Before this change, a subshell or substitution that hit the step or output limit stopped only its own child. The parent continued, and a run that had hit a limit could exit 0.

## Decision
- **One limit, `MAX_COMMAND_DEPTH = 32`,** counted in the parser for compound commands and in the interpreter for every command that runs inside another: function calls, subshells, substitutions, loop bodies, and groups. Substitution nesting keeps its own limit of 32 (`MAX_NESTING`).
- **The limit is measured, not chosen by feel.** A debug build overflows a 1 MiB stack at 64 levels and runs 32 levels in it. The 2 MiB stack of a tokio worker therefore leaves at least 2x headroom at 32. A release build uses less stack, so the margin is larger there.
- **A limit error is an error, not a crash.** The parser returns `ResourceExhausted`. At run time the command fails with exit status 1 and the message `commands nested deeper than 32 levels`, and the run aborts.
- **A child's limit aborts its parent.** A subshell or substitution that hits a limit copies `Abort` back to the parent, through one helper, `absorb`. Previously only the step count and the output count came back.
- **An aborted run fails.** `run` and `run_argv` report exit status 1 when the run was aborted, even if the last command succeeded.

## Consequences
### Positive
- No script can overflow the stack of the process it runs in, through recursion or nesting.
- A run that hit a limit cannot report success.
- The fuzzer's input is a regression test (`crates/sandbox-shell/tests/recursion.rs`), and each recursive path has its own test on a 2 MiB stack.

### Negative
- A script can nest at most 32 compound commands or calls deep. Recursive shell functions deeper than 32 levels now fail. Real agent scripts do not come close, but a deep recursive walk of a directory tree does not fit.
- The limit is a constant, not a `ShellConfig` setting. A caller cannot raise it per session. Raising it needs a bigger stack first.

### Risks
- The margin was measured on this machine, with a debug build and a 1 MiB stack. A platform with larger frames, or a call path that adds frames the test does not cover, would use more stack. Re-measure before raising the limit.

## Alternatives Considered
- **Run the shell on a dedicated thread with a large stack.** Deferred. It moves the risk rather than bounding it, and a 1 MiB stack can still overflow under a deeper call path. Revisit when the shell has a sandboxed worker (PLAN §4.13).
- **Bound depth only by the step budget.** Rejected. The step budget ran out only after the stack was gone.
- **Make the limit a `ShellConfig` field.** Deferred until there is a caller that needs a different value.

## References
- [PLAN.md §5](../../PLAN.md): the security model, and the requirement to fuzz the shell parser.
- [ADR 0013](0013-blob-store-garbage-collection.md) and [ADR 0017](0017-content-defined-chunks.md): the other changes in this session.
- Fuzz target `fuzz/fuzz_targets/shell_run.rs`, and the crash it found.
