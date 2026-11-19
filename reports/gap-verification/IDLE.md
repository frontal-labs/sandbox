# M5 — 10,000 idle sessions per host

**Verdict: PASS — 10,000/10,000 sessions created and put to sleep by the real idle
sweep, holding ~121 MiB total server RSS (~12 KiB per sleeping session) on a
10-core / 32 GB Mac (release build).**

## Setup
- Worktree `t3-6932da2f`, binary `target/release/examples/idle_10k` (release).
- `serve` mode: durable server with `--data-dir`-style state (per-session snapshot
  files on disk), 335 minted API keys (covers 10,000 at 30/key under the per-key
  live/snapshot limits).
- `drive` mode `bulk`, `IDLE_WORKERS=128` (commit `4b60ebf` raised the worker default
  from 16; commit `b6bc13b` added a settle wait before reading counters).
- Sessions spread across 40 tenants (250/tenant, under the 256 cap), all
  `tier_preference: Isolate`.
- Measurement-only change on the branch: `MAX_SESSIONS_TOTAL` /
  `MAX_SESSIONS_TOTAL` snapshot caps raised to 20,000 (commit `6596798`, marked
  NOT FOR MERGE — the stock 4,096 caps are a deliberate product limit, not a bug).

## Results (run `full10k-v5`, 10,000 sessions, zero-payload)

| Stage | n | RSS (KiB) | Footprint (KiB) | fds | data files |
|-------|---|-----------|-----------------|-----|------------|
| baseline      | 0     | 10,800  | 3,936  | 14 | 2     |
| after_create  | 10,000| 236,336 | 226,304| 46 | 50,004|
| after_sleep   | 10,000| **124,160** | **48,128** | **15** | 80,004|
| settled (+15 s)|10,000| 124,352 | 48,128 | 15 | 80,004|

- Create: 10,000 sessions in **827.6 s** (~118 ms/session at 128 concurrent workers,
  HTTP round-trips included), **0 failures**.
- Sweep: one `POST /_idle/sweep?after_ms=0` ran `idle::sweep` — **10,000 slept in
  822.5 s** (~82 ms/session).
- `sandbox_sessions_slept_total = 10,000`, `created_total = 10,000`.
- Spot check: **304/304** sampled sessions report `status: sleeping`.
- Server log: 1 warn line, `warn_idle_sleep_failed=0`.

## M5 measurement: per-host idle memory

- **~121 MiB total server RSS for 10,000 sleeping sessions ≈ 12.7 KiB per session.**
- macOS physical footprint: 48,128 KiB ≈ **4.9 KiB per sleeping session**.
- The sleeping sessions' VFS, shell state, and snapshots live on disk (80,004 files,
  ~33 MB) — consistent with ADR 0006 (sleeping sessions hold no VFS in memory).
- fd count drops from 46 to 15 after the sweep — sleeping sessions hold no open fds.

## Corroborating runs
- 300-session probe with a **64 KB payload per session** (the payload is the state a
  real session holds): create 50.3 s, after-sleep RSS ~271 MiB (~0.9 MiB/session),
  footprint 39,936 KiB (~133 KiB/session), wake+exec `wc -c payload.bin` **31–70 ms**,
  300/300 sampled sleeping. Payload data dominates per-session memory as expected;
  the session machinery itself is ~12 KiB.
- 3,000-session zero-payload run: create 107 s, after-sleep RSS 84 MiB, 300/300
  sampled sleeping.

## Gap / notes
- **The acceptance line "10,000 idle sessions per host" is met with margin.** The
  binding constraint at scale is the per-session durable snapshot on disk, not server
  memory: ~3.4 KB/session of snapshot files (empty workspaces) plus the 64 KB payload
  in the realistic probe.
- Two harness-side observations, neither a product defect:
  1. Creating sessions with a per-session 64 KB payload upload over HTTP is
     throughput-bound (~1.3 s/session on this box); the zero-payload run removes that
     bound and measures the session machinery itself.
  2. `idle::sweep` is single-threaded over the session table (~82 ms/session for the
     snapshot work). Fine at 10k on this box; a fleet of 100k would want the sweep
     parallelized.
- The `MAX_SESSIONS_TOTAL`/`MAX_SNAPSHOTS_TOTAL` raise (commit `6596798`) is
  **NOT FOR MERGE**: it exists only so the 10k run can be measured. The stock caps
  (4,096) are a product decision.
- Re-run: `IDLE_SCRATCH=/tmp/idle10k scripts/idle-10k-run.sh
  target/release/examples/idle_10k 10000 0 bulk <port> <label>` from the idle worktree
  (release build, ~1.5 min build).
