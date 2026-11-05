# Service levels and measurements

This file states the targets the server is measured against, and the numbers the capacity
example (`crates/sandbox-server/examples/capacity.rs`) measured for them. Two kinds of target
appear here:

- **Plan targets** come from `plans/PLAN.md`. They are binding for the milestones that state them.
- **Proposed targets** are not in the plan. They are proposals for the owner to accept or change.
  They are marked as such and are not treated as met or unmet until they are agreed.

## Measurements

Release build, one process, one client issuing requests in sequence, on a single macOS
development machine. Each latency is the time one HTTP request takes through the router, with the
server in the same process. The numbers are from 2026-11-05. They vary between runs, and the
durable numbers vary most, because they depend on the disk.

| Operation | In memory p50 / p95 | Durable p50 / p95 |
|---|---|---|
| Snapshot | 0.026 / 0.041 ms | 40 / 42 ms |
| Fork (one session) | 0.106 / 0.125 ms | 37 / 40 ms |
| Create session | 0.067 / 0.086 ms | 28 / 30 ms |
| Exec `echo` | 0.048 / 0.058 ms | 23 / 25 ms |

Memory per session, measured as resident-set growth:

| State | In memory | Durable |
|---|---|---|
| Idle live session | 8.4 KiB | 86 KiB |
| Sleeping session, in the same process | 10.7 KiB above idle | 93 KiB above idle (see below) |
| Sleeping sessions after a restart, each with its snapshot | not measured | ~0 per session; a fixed ~1.7 MB at open |

The restart figure is from `examples/parked_restart`, which runs each step in its own process:
an empty directory opens at about 4,400 KiB, and 1,000 sleeping sessions, each with a snapshot,
open at about 6,100 KiB. See ADR 0034.

## Targets

| Target | Kind | Requirement | Measured | Status |
|---|---|---|---|---|
| Snapshot and fork under 5 ms | Plan (M3 accept) | p95 < 5 ms | in memory 0.04 / 0.13 ms; durable 42 / 40 ms | met in memory; **not met durable** |
| Sleeping sessions use no memory | Plan (M3 accept) | 0 per sleeping session | ~0 per session after restart; fixed ~1.7 MB at open | met per session; fixed open cost remains |
| Create session | Proposed | p95 < 50 ms | in memory 0.09 ms; durable 30 ms | met |
| Exec a command | Proposed | p95 < 50 ms, echo case | in memory 0.06 ms; durable 25 ms | met |
| Idle session memory | Proposed | < 100 KiB | in memory 8.4 KiB; durable 86 KiB | met |

## Open items

- **Durable snapshot and fork are fsync-bound.** Each record and each link is written with a
  synced write, so one snapshot costs about 40 ms. That misses the plan's 5 ms target on a durable
  server. The fix is to batch the syncs, which weakens what a crash can lose; that trade-off needs
  the owner's decision before it is made.
- **Concurrency is not measured.** These numbers are sequential requests. The plan's load target
  (1,000 concurrent sessions on one node) is not measured here.
- **The durable numbers are from one disk.** They should be measured again on the production
  storage before a target is agreed.
