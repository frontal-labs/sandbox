# Milestones

Based on the project specification section 9. A box is ticked only when the code
clearly implements the item and a test exists for it. Items that are scaffolded
but not working, or implemented without tests, stay unticked and are marked
**Partial**. Acceptance criteria are not ticked unless they were demonstrated.

Last reviewed against the source tree on 2026-11-21.

## M0: Foundations (Week 1)

**Goal**: Cargo workspace, CI, `sandbox-types`, `sandbox-cas` (Mem + Disk)

### Deliverables
- [x] Cargo workspace with all 18 crate members present (`Cargo.toml`)
- [x] CI pipeline: fmt, clippy `-D warnings`, test, and a
      `supply-chain` job (cargo-deny, cargo-audit) are in `ci.yml`. The
      GitHub run is green on every pull request and main push
      (confirmed 2026-11-14). The KVM VM workflow
      (`.github/workflows/vm-kvm.yml`) runs the `--ignored` VM suites on
      main pushes and pull requests.
- [x] `sandbox-types`: IDs, errors, events, config, and `serde` are present.
      The OpenAPI spec comes from `utoipa` behind the `openapi` feature (ADR 0001).
      `schemars` was removed as unused (PLAN §4.1)
- [x] `sandbox-cas`: `BlobStore` trait, `MemStore`, `DiskStore`, `TieredStore`
- [x] CAS property tests (proptest) in `crates/sandbox-cas/src/disk_store.rs`:
      every blob round-trips, an insertion stores only the chunks around it, and
      a sweep keeps what live blobs name

### Acceptance Criteria
- [x] CI green on all platforms. The GitHub run is green on every pull
      request and main push (confirmed 2026-11-14)
- [x] CAS property tests pass (`cargo test -p sandbox-cas`, 2026-10-19)
- [x] `cargo deny check` passes (`deny.toml`, 2026-10-19)
- [x] `cargo audit --deny warnings` passes (`.cargo/audit.toml`, 2026-10-19)

### Status: **Done**

---

## M1: Isolate + VFS + Shell Core (Weeks 2-5)

**Goal**: Tier 1 execution with shell, VFS, V8 runtime

### Deliverables
- [x] `sandbox-vfs`: copy-on-write overlay, Merkle commit, symlinks, quotas,
      proptest model checks.
- [x] `sandbox-runtime`: `execute_script` runs JavaScript in a fresh V8 isolate with
      `console.log` captured, a wall-clock timeout, an output cap, a heap cap,
      and a `node:` module shim (fs, env, process, buffer, path, url, crypto,
      os, events, stream, util, assert, string_decoder, querystring). Network,
      ESM, child_process, and zlib are intentionally unimplemented (see
      `sandbox-runtime` crate doc).
- [x] `sandbox-shell`: parser (quotes, `|`, `&&`, `||`, `;`, `<`, `>`, `>>`),
      interpreter, VFS-backed builtins, variables, globs, and `/bin` WASI tools.
      `case`, `$((...))`, parameter operators, and fd redirection are implemented.
      Host binaries are not. Recursion is bounded at 32 levels, and a limit aborts
      the run (ADR 0018). Fuzz targets exist for the parser, the interpreter, VFS
      paths, and the WASI fs shim (`fuzz/`).
- [x] `sandbox-session`: `IsolateBackend` owns a VFS and runs commands through
      the shell. Snapshot, restore, and sleep (checkpoint) work; the server
      releases a sleeping session's VFS (ADR 0006).
- [ ] **Partial.** `sandbox-sdk` (Rust): embedded `exec` and `create_session`
      return `Unsupported`; the in-process path is `sandbox-session` directly
- [x] `cli` (`sandbox run`, `sandbox shell`): execute in an isolate session

### Acceptance Criteria
- [x] `sandbox run "echo hi | cat > /a && cat /a"` works
- [x] `node -e`, `node FILE.js`, and `node FILE.ts` run against VFS (ADR 0024: TypeScript by type stripping; types are not checked)
- [x] Cold start < 5 ms from warm pool in `bench/`: a new Tier 1 session takes a p50 of 833 ns, and a `node` run takes a p50 of 0.41 ms from a warm isolate (5.10 ms cold). The back-to-back tail is the cold cost (ADR 0025)
- [x] 200+ shell conformance cases pass (219 of 219 against GNU bash 5.3.8, ADR 0019)

### Status: **Partial**

---

## M2: WASM Tools + Node Compat + Trace (Weeks 6-9)

**Goal**: WASM tooling, Node.js compatibility, tracing

### Deliverables
- [x] `sandbox-wasm`: WASI preview1 command modules run with in-memory stdio, argv,
      fuel, a memory cap, a VFS-backed preopen, and a tool registry.
- [ ] Tools: `jq`, `ripgrep`, `sqlite3`, `git`, `python`
- [ ] Coreutils to 40 commands
- [x] `node:` shim (`fs`, `path`, `os`, `events`, `stream`, `util`, `buffer`, `url`, `crypto`, `env`, `process`, `assert`, `string_decoder`, `querystring`). `child_process` and `zlib` are stubs that throw; network and ESM are intentionally unimplemented
- [x] `sandbox-trace`: trace format, recording from `run` and `shell`, and replay (ADR 0007). The event log in `log.rs` is still unused. Early

### Acceptance Criteria
- [ ] `git clone` (via allowed host), commit, diff works
- [ ] `python -c` and `pip`-free stdlib scripts run
- [x] Trace shows every file change: commands record their file changes (ADR 0011), and writes and deletes through the filesystem API are runs of their own (ADR 0012). Renames and chmod are not separate operations, so a rename shows as a delete and a create
- [ ] 70% pass rate on first agent-transcript corpus

### Status: **Partial**

---

## M3: Server, Policy, Snapshots (Weeks 10-13)

**Goal**: Multi-tenant server with policy, snapshots, scheduling

### Deliverables
- [x] `sandbox-policy`: capability, limits, and network modules exist, with
      integration tests (16)
- [x] `sandbox-snapshot`: removed. The in-memory `SnapshotManager` was dead
      scaffolding; the server owns snapshot state in its own table (ADR 0049)
- [x] `sandbox-worker`: one child process per session, gRPC over a private Unix socket, with
      crash isolation. On Linux the child gets namespaces, a seccomp allowlist, `NO_NEW_PRIVS`,
      and cgroup v2 limits (ADR 0022); each layer fails closed. `cli/tests/worker_isolation.rs`
      verifies all of it from outside the process.
- [ ] **Partial.** `sandbox-server`: session, exec, filesystem, snapshot, and fork routes at `/v1` run on in-memory sessions behind API-key auth, with OpenAPI (utoipa), 429 + Retry-After on session admission refusals (T-08), and file-backed billing (T-03)
- [ ] **Partial.** `sandbox-scheduler`: placement, pools, and scheduler modules
      exist, with integration tests (15); sleep/wake and metering not verified

### Acceptance Criteria
- [ ] Snapshot/fork under 5 ms (in memory met; durable not met — see SLO.md)
- [x] 1,000 concurrent sessions on one node in load test. Verified in
      `reports/gap-verification/LOAD.md`: 1,000/1,000 admitted, 0 errors,
      4,898 ops/s sustained exec, p99 5.35 ms, no ceiling observed. Harness:
      `benchmarks/load` + the `load_node` example.
- [x] Sleeping sessions hold no VFS in memory (ADR 0006). With `serve --data-dir`, live and sleeping sessions, their shell state, their traces, their snapshots, and the keys survive a restart (ADRs 0008, 0010).
- [ ] Escape test suite passes (runtime, shell, and WASM suites pass natively
      and in the Linux Docker harness; the OS-level containment suite —
      `cli/tests/contained_escape.rs` — is not built yet, so the kernel-side
      tier is uncovered. See `reports/gap-verification/SECURITY.md`)
- [ ] TS SDK generated from OpenAPI (current TS SDK is hand-written)

### Status: **Partial**

---

## M4: Tier 2 (Weeks 14-19)

**Goal**: QEMU microVMs, promotion, guest agent

### Deliverables
- [x] **Working.** `sandbox-vmm`: QEMU driver (`QemuVm`), warm pool (`VmPool`), snapshot/restore via QEMU monitor. The Firecracker driver and jailer were scaffolding and were removed (T-05).
- [x] **Working.** `sandbox-guest`: agent implementing `GuestService` over gRPC, booted as init in the emulated test guest
- [ ] Base + browser rootfs images
- [x] `import_from_dir` for promotion (recovers an ext4 image into the host VFS); `export_ext4` (host VFS to image) is the round-trip counterpart
- [x] `sandbox-promote`: `promote_to_vm` implemented, wired into the server for `vm`-tier sessions. The automatic `NeedsVm` re-promotion of an existing isolate session is not built.
- [x] VM warm pool, QEMU snapshots (`savevm`/`loadvm`)

### Acceptance Criteria
- [ ] `ffmpeg` and headless Chromium run via auto-promotion
- [ ] Promotion under 400 ms
- [ ] VM resume < 150 ms
- [ ] Guest protocol fuzzed for 24h clean

### Status: **Partial**

---

## M5: Production Hardening (Weeks 20-24)

**Goal**: Clustered deployment, security review, operations

### Deliverables
- [ ] Clustered state with Postgres: the metadata store is built and tested against Postgres 18 (ADR 0023). The clustered scheduler is not built, and blobs still need a shared directory
- [ ] Object-store CAS (S3-compatible)
- [ ] CAS GC from live roots
- [ ] Multi-tenant quotas, billing export
- [ ] Dashboards, runbooks
- [ ] External security review (not yet performed; status recorded in
      [SECURITY_REVIEW.md](SECURITY_REVIEW.md))

### Acceptance Criteria
- [ ] Chaos suite passes. The file-backed suites `chaos_crash`, `chaos_damaged`,
      and `chaos_torn` are plain (not `#[ignore]`-gated) tests, so they run in the
      `rust` job of `ci.yml` on every push to `main` and every pull request
      targeting `main`. The clustered suite `chaos_clustered.rs` builds only with
      `--features clustered` (which that job does not enable) and needs
      `SANDBOX_TEST_DATABASE_URL`, so it is not in any CI job: it passed against
      Postgres 16 in `reports/gap-verification/CHAOS.md` (re-run with `cargo test
      -p sandbox-server --features clustered --test chaos_clustered`), and the
      local clustered runner is `docker/clustered-test.sh`. The full clustered
      story is still incomplete: no multi-node scheduler, S3 CAS, or clustered
      billing log.
- [x] 10,000 idle sessions per host. Verified in
      `reports/gap-verification/IDLE.md`: 10,000/10,000 created, the real idle
      sweep puts all 10,000 to sleep, ~121 MiB total RSS (~12.7 KiB/session).
- [x] Documented SLOs. `docs/SLO.md` records plan targets (snapshot/fork < 5 ms
      in memory, met; durable not met) and proposed targets (create < 50 ms, exec
      < 50 ms, idle < 100 KiB, all met) with measured values.
- [ ] Security review findings resolved (no review performed yet — see
      [SECURITY_REVIEW.md](SECURITY_REVIEW.md))

### Status: **Partial**

---

## M6: Developer Experience (Ongoing)

**Goal**: Polish, examples, integrations

### Deliverables
- [ ] Interactive PTY shell
- [x] `sandbox replay` CLI, with recording through `run --trace` and `shell --trace`
- [x] Snapshot templates: named, key-scoped templates over saved snapshots
      (`POST/GET /v1/templates`, `initial_template` on session creation,
      persisted across restarts). ADR 0049. Preset images such as `node-20`
      and `python-3.12` are not built in yet
- [ ] VS Code / MCP server
- [ ] Docs site
- [ ] Examples: coding agent, data agent, browser agent

### Status: **Partial**

---

## Definition of Done (v1)

- [ ] All milestones M0-M5 accepted
- [ ] Public API stable and documented
- [ ] OpenAPI + Rust SDK + TS SDK published
- [ ] Benchmarks meet Section 6 targets on reference hardware
- [ ] Security review complete, no open high-severity findings
- [ ] Three working examples + one-page quickstart (< 2 min to running `exec`)

---

## Progress Tracking

| Milestone | Target Week | Status | Blockers |
|-----------|-------------|--------|----------|
| M0 | 1 | Done | CI green on all PRs and main pushes (confirmed 2026-11-14); KVM VM workflow runs `--ignored` suites. The push-only Release workflow is outside that confirmation: it first ran 2026-11-15 and failed to parse on a YAML syntax error in `release.yml` (an unquoted `:all:`), fixed on 2026-11-15 |
| M1 | 2-5 | Partial | Shell and isolate `exec` work over the VFS; 219 conformance cases pass against GNU bash (acceptance: 200+ cases). `node -e`, `.ts` against VFS, and the cold-start benchmark are still open |
| M2 | 6-9 | Partial | WASI command modules run over a VFS preopen, with a tool registry; no persistent AOT cache, no preview2 |
| M3 | 10-13 | Partial | Session, exec, fs, snapshot, and fork routes run in memory behind API-key auth with OpenAPI and 429 + Retry-After; worker runs one child process per session. 1,000-concurrent and SLO targets verified (see `reports/gap-verification/`). Durable snapshot latency and the OS-level escape tier are the open items |
| M4 | 14-19 | Partial | QEMU VMM with warm pool, guest agent, promote_to_vm, import_from_dir/export_ext4 all working; auto-promotion and the 400 ms target measured in `reports/gap-verification/AUTOPROMOTE.md` but not yet in CI; rootfs images not done |
| M5 | 20-24 | Partial | Postgres state store (one node at a time, conditional writes, tested on Postgres 18). 10,000-idle verified and SLOs documented; clustered scheduler, placement, object-store blobs, and the external security review are the open items |
| M6 | Ongoing | Partial | `sandbox replay` and snapshot templates (ADR 0049) are done; PTY shell, VS Code/MCP server, docs site, and examples are not started |

---

## Notes

- Work milestone by milestone. Do not start a milestone until the previous one's acceptance criteria pass.
- Start every milestone by writing the public trait/types and tests, then implement.
- Keep crates small and dependencies one-directional.
- Public API changes need an ADR in `docs/adr/`.
- All code must pass `cargo fmt`, `cargo clippy -- -D warnings`, and tests before commit.
