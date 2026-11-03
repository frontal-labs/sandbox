# 0033: CPU accounting for a command

## Status
Accepted

## Date
2026-11-03

## Context
`ExecResult.cpu_ms` was always 0. The shell path is async, and a command can move between
runtime threads at any await point, so one clock read at the start and one at the end
measures the wrong thing. The plan (section 4.11) names CPU-ms as a metered quantity, and
ADR 0014 accepts a `cpu_ms_per_sec` limit that cannot be enforced without a measurement.

Options considered: wall time (already reported as `wall_ms`, and it counts waiting), cgroup
statistics (a worker serves many sessions, so they cannot be split per command on the async
path), fuel and instruction counts (exact for WASM tools, but they are not time and do not
apply to the shell), and per-poll thread CPU time.

## Decision
- `cpu_ms` is the CPU time of the calling thread, summed over every `poll` of the command's
  future. A `CpuMeter` wrapper reads the thread CPU clock (`CLOCK_THREAD_CPUTIME_ID`)
  before and after each poll. The total is exact when the runtime moves the task between
  threads, because each poll runs on one thread.
- Waiting is not CPU time. A command waiting on a lock, a timer, or I/O uses no CPU while it
  waits, so it adds nothing.
- WASM tools run inside the shell's poll, so their CPU counts toward the command. A tool that
  runs on another thread would not be counted, and no tool does that today.
- `cpu_ms` is an upper bound on the time the command held a thread, and at most its wall
  time on a single command, as the test checks. It is not a billing figure for the host:
  hardware and scheduler effects are not separated.
- `cpu_ms_per_sec` is not yet enforced against this measurement. The limit check is still
  the zero-value check from ADR 0014. Enforcement is the next step, and it needs a per-session
  budget that a command can exhaust.

## Consequences
### Positive
- Usage can be metered per command, and the number has the meaning the field name says.
- The measurement needs no privileges and works on macOS and Linux.

### Negative
- A thread CPU clock read costs a system call on every poll. The shell polls at each
  command, so a long loop adds one read per command.
- Time on other threads, such as the WASM host's own threads if one is added, is missed.

## Alternatives Considered
- Wall time only. Rejected: it counts waiting, which is not compute.
- cgroup statistics per worker. Rejected for now: a worker serves many sessions, and the
  worker tier is not the path that runs the shell.
- Fuel as CPU time. Kept for WASM tools as a budget, not as a time measure.

## References
- ADR 0014 (per-session limits), ADR 0030 and 0032 (WebSocket exec), PLAN.md §4.11 (metering).
