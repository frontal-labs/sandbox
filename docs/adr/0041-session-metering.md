# 0041: Per-session CPU and memory metering

## Status
Accepted

## Date
2026-11-11

## Context
PLAN §4.14 asks for metering of CPU time and memory, and the usage export (ADR 0021, `GET
/v1/usage/export`) carried only per-key counters with no CPU or memory. CPU time is measured per
command (ADR 0033). Memory was not recorded at all.

## Decision
- Each session has a meter (`sandbox-server/src/metering.rs`). It holds the number of commands,
  CPU time, wall time, memory-time in byte-milliseconds, and the peak memory seen.
- A meter opens when a session is admitted, so an idle session is reported with zeros.
- After each command that finishes successfully, the server adds the command's CPU and wall
  time, and the session's filesystem size (`SessionFs::total_bytes`) weighted by the wall time.
  A failed command is not metered, because it has no result to read.
- The usage export carries a `sessions` list for the caller's keys, one entry per session with
  its meter. Each snapshot line in `usage.jsonl` therefore keeps the per-session history too.
- "Memory" here is the session's filesystem size, the quantity its VFS quota limits. It is not the
  process's resident memory, which the server does not attribute per session. That is a separate
  measurement, still open.

## Consequences
### Positive
- Every session produces a record with CPU and memory values, including idle sessions.
- The arithmetic is pure and covered by unit tests. A clustered test (`clustered_metering.rs`)
  checks the export against a live Postgres and an S3-compatible service, through
  `docker/clustered-test.sh`.

### Negative
- Memory is sampled at command boundaries. Filesystem growth between commands is not counted
  until the next command finishes, and a session that writes and sleeps is metered at its last
  command's size.
- Meters live in memory and reset on restart, as the per-key counters do. The durable record is
  the snapshot history in `usage.jsonl`, which is kept only on a durable server.

## Alternatives Considered
- Sample process memory per session. Not chosen: sessions share the process, so there is no
  per-session RSS to read without a worker per session (PLAN §4.13).
- Charge memory per second of session lifetime. Not chosen for now: it needs a clock for every
  live session, and the command-boundary sample is enough for the current billing period model.

## References
- PLAN §4.14, §4.13. ADR 0021 (usage export), ADR 0033 (CPU accounting), ADR 0023 (clustered state).
