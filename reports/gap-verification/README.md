# Gap verification — final summary

Five production-readiness gap items, tested on a 10-core / 32 GB Apple Silicon Mac,
release builds, against the tree at `9ed79b5`. Each track ran in its own worktree
(now removed; branches preserved for inspection).

| # | Item | Verdict | Headline number |
|---|------|---------|-----------------|
| 1 | M3 — 1,000 concurrent sessions | **PASS** | 1,000/1,000 admitted, 0 errors, 4,898 ops/s, p99 5.35 ms |
| 2 | M5 — 10,000 idle sessions/host | **PASS** | 10,000 slept, ~121 MiB total RSS (~12 KiB/session) |
| 3 | M4 — ffmpeg + Chromium auto-promotion | **PASS** | e2e 63 s with real binaries; small-workspace promotion 9 ms vs 400 ms target |
| 4 | M5 — clustered chaos suite | **PASS (4/4)** + 1 real bug found | suite now exists and passes; clustered build was broken before |
| 5 | M3+M5 — escape suite + hardening | **PASS for built tiers**; OS-level tier **UNBUILT** | 56 pass / 0 fail / 17 known-gap ignores |

All results: no existing test was modified to make a failure pass; every green result
is a new test against current behavior.

## Track branches (all cut from `9ed79b5`)

| Track | Branch | Head |
|-------|--------|------|
| load | `t3/02b72f02` | `64d8d3c` |
| idle | `test/idle-10k-measure` | `b6bc13b` |
| autopromote | `autopromote/m4-hvf-arm64-guest` | `ffa209f` |
| chaos | `t3/25a2e45a` | `3c94f20` (bug report `a7b52e5` is its parent) |
| security | `t3/1aefc3d6` | `9694507` |

## Per-track details

### 1. M3 — 1,000 concurrent sessions — PASS
Ran the ramp beyond the brief (50 → 100 → 1,000) against a release `load_node`
server with 40 keys; the 1,000-session run supersedes the intermediate stages.

- **1,000/1,000 admitted in 9.99 s, 0 create failures.**
- Sustained exec at 1,000 concurrent (200 ms think time): **4,898 ops/s**, 0 errors
  over 100,885 iterations.
- Exec latency flat across the ramp: mean 1.30 → 1.42 ms, p99 10.04 → 5.35 ms,
  max 10.3 ms. No knee observed.
- Server RSS: 55 → 111 MiB (+~0.11 MB/session; sessions are thin handles).
- **No ceiling observed at 1,000.** Acceptance line met with margin.
- New reusable harness: `benchmarks/load` + `load_node` example (committed).

### 2. M5 — 10,000 idle sessions per host — PASS
Release `idle_10k` (new example on the idle branch): a durable server whose own
RSS is read from outside, sessions put to sleep by the **real idle sweep**
(`idle::sweep`), not `/sleep`.

- 10,000/10,000 created (0 failures, ~118 ms/session over HTTP at 128 workers);
  one sweep put **all 10,000 to sleep** (~82 ms/session); 304/304 sampled
  `sleeping`; `warn_idle_sleep_failed=0`.
- **M5 measurement: ~121 MiB total server RSS for 10,000 sleeping sessions
  ≈ 12.7 KiB/session** (footprint 4.9 KiB/session). fds drop 46 → 15. VFS and
  snapshots on disk (80,004 files, ~33 MB) — consistent with ADR 0006.
- Realistic probe (300 sessions, 64 KB payload each): after-sleep ~0.9 MiB/session,
  wake+exec 31–70 ms. Payload dominates; machinery is ~12 KiB.
- Caveats: the `MAX_SESSIONS_TOTAL`/`MAX_SNAPSHOTS_TOTAL` raise to 20,000 is
  **NOT FOR MERGE** (measurement only); per-session 64 KB payload upload is
  throughput-bound over HTTP (~1.3 s/session) — that is the harness, not the server;
  the sweep is single-threaded (fine at 10k, parallelize for 100k).

### 3. M4 — ffmpeg + headless Chromium auto-promotion — PASS
- `vm_auto_promote_real` (e2e, release, guest built by the new
  `docker/build-guest-image.sh`, booted under QEMU **HVF**, aarch64): a session
  execs an ffmpeg transcode and a headless Chromium render; the server
  auto-promotes the isolate session to a microVM and both complete in the guest.
  **1 passed / 0 failed, 63.02 s.**
- `promotion_latency` (the "400 ms" number), warm `VmPool`, release, HVF:
  empty 0 ms, **8×4 KiB 9 ms**, 200×8 KiB 261 ms, 1×32 MiB 644 ms.
  The 400 ms assertion (small tree, KVM/HVF) → **9 ms, PASS.**
- Environment note: tests are `#[ignore]`-gated on `SANDBOX_VM_IMAGE`; the Docker
  guest-image build works on Apple Silicon (HVF). Out of scope here: VM resume
  < 150 ms and the 24 h guest-protocol fuzz.

### 4. M5 — clustered chaos suite — PASS (4/4), plus a real bug
- **New** `crates/sandbox-server/tests/chaos_clustered.rs` (commit `3c94f20`):
  atomic save / crash leaves previous document; damaged body refused on open;
  wrong version refused; stale node cannot overwrite a newer revision.
  **4 passed / 0 failed, 1.75 s** against live `postgres:16`. Gated on
  `--features clustered` + `SANDBOX_TEST_DATABASE_URL`.
- **Real bug found (commit `a7b52e5`, NOT FOR MERGE as-is):** with
  `--features clustered`, `open_durable_with_options` called the file-backed
  `BillingLog::open` (which is `cfg(not(clustered))`) unconditionally — so the
  crate **did not compile** under the clustered feature (introduced in `9a66fc0`,
  #108). The gate makes it compile; the right fix (shared/Postgres-backed durable
  billing log) is a design decision.
- Still missing for the full M5 clustered story (out of scope): multi-node
  scheduler, S3-backed CAS, clustered billing log.

### 5. M3+M5 — escape suite + security hardening — PASS for built tiers; OS tier UNBUILT
- **New** escape suites, one per execution tier, run natively and in a privileged
  Linux Docker container (`docker/escape-linux.Dockerfile`,
  `docker/run-escape-tests.sh`):

  | Suite | Pass | Ignore (known gaps) | Fail |
  |-------|------|---------------------|------|
  | WASM   | 19 | 6 | 0 |
  | SHELL  | 24 | 3 | 0 |
  | RUNTIME| 13 | 8 | 0 |
  | **Total** | **56** | **17** | **0** |

- Ignores are explicit known gaps (resource-controller memory probes, OS-level
  namespace checks), each with a reason.
- **The gap:** the OS-level contained tier is unbuilt — `cli/tests/contained_escape.rs`
  and `sandbox-worker/tests/escape_linux.rs` don't exist (the Docker script fails on
  the worker tier for that reason), and `docs/ESCAPE_TESTS.md` is missing. Consistent
  with MILESTONES M1: seccomp/cgroups/namespaces in `sandbox-worker` are not
  implemented. "Escape test suite passes" is satisfied for the suites that exist;
  the kernel-side containment they defend is not there yet.

## Infra incident (for the record)
All five subagent runs initially died on `SQLITE_FULL` — T3 Code's server DB filled
the disk (each worktree's `target/` ~21 GB total). Freed 22 GB, restarted Docker,
committed all subagent work to its branch, and completed every track by direct
re-runs. Nothing in the final results is affected.

## Artifacts
- Reports: `reports/gap-verification/{LOAD,IDLE,AUTOPROMOTE,CHAOS,SECURITY}.md`
- Logs: `/tmp/load-node.log`, `/tmp/idle10k/full10k-v5.log`, `/tmp/autopromote.log`,
  `/tmp/promotion-latency.log`, `/tmp/escape-linux.log`
- Guest image: `/tmp/guest-image/` (aarch64, vmlinuz + initrd.gz)
- Baseline re-confirmed before the work: `cargo test --workspace` 653 pass / 0 fail.
