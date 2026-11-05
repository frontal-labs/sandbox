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

## Tier 2 (QEMU, KVM)

From the `VM tests (KVM)` job on GitHub's Ubuntu 24.04 runner, commit 04f7ab3, which passed. The
timings test prints one line per run:

| Measure | Value |
|---|---|
| Cold boot of a guest, until it answers `Ping` | 7.6 s |
| Warm handout from the pool | 0 ms |
| Promote one 64 KiB file into a guest | 5 ms |
| CPython runs in a promoted guest (whole test, including boot) | 141 s |

These are the first numbers from hardware virtualisation. The emulated (TCG) figures are not
recorded here, because the Python test takes over 25 minutes under emulation.

## Decisions

These were open, and the owner asked for them to be decided here.

- **Durable snapshot fsync: keep the synced writes.** A snapshot is about 40 ms on a durable
  server, against 5 ms for the plan's target. Weakening the sync would let a crash lose an
  acknowledged snapshot, and the plan promises durability for them. Speed is the cost, and it is
  accepted. Revisit only with a measured reason that a faster disk or a batched commit can meet the
  guarantee.
- **Resume speed: do not put saved states in the promotion pool yet.** The pool already hands out a
  booted guest in 0 ms, so saved states would only shorten refills. Under emulation a resume takes
  12 to 14 s against a cold boot of 19 to 31 s, so it helps, but it is far from 150 ms. The 150 ms
  criterion is not met, and it is unmeasured on KVM. Revisit when a KVM resume is measured.
- **Four-byte residual in `python-init.wasm`: leave it unpinned.** The conformance cases and the
  install-time `reactor_hash` cover the reactor's behaviour, and the cause is probably a clock
  reading, which only the build's Wizer step captures. More build cycles would not change what the
  reactor does, so the residual stays documented.
- **Object-store CAS: accept `adobe/s3mock` as the S3 test target.** The code uses only the S3 API,
  and MinIO's images could not be pulled from Docker Hub or Quay on 2026-11-05. Run the same tests
  against MinIO when its images are reachable; that is a follow-up, not a blocker.

## Open items

- **Durable snapshot and fork are fsync-bound.** Each record and each link is written with a
  synced write, so one snapshot costs about 40 ms. That misses the plan's 5 ms target on a durable
  server. The fix is to batch the syncs, which weakens what a crash can lose; that trade-off needs
  the owner's decision before it is made.
- **Concurrency is not measured.** These numbers are sequential requests. The plan's load target
  (1,000 concurrent sessions on one node) is not measured here.
- **The durable numbers are from one disk.** They should be measured again on the production
  storage before a target is agreed.
