# Milestones

Based on [plans/PLAN.md](../plans/PLAN.md) section 9. A box is ticked only when the code
clearly implements the item and a test exists for it. Items that are scaffolded
but not working, or implemented without tests, stay unticked and are marked
**Partial**. Acceptance criteria are not ticked unless they were demonstrated,
and none has been demonstrated yet.

Last reviewed against the source tree on 2026-10-19.

## M0: Foundations (Week 1)

**Goal**: Cargo workspace, CI, `sandbox-types`, `sandbox-cas` (Mem + Disk)

### Deliverables
- [x] Cargo workspace with all 18 crate members present (`Cargo.toml`)
- [ ] **Partial.** CI pipeline: fmt, clippy `-D warnings`, test, and a
      `supply-chain` job (cargo-deny, cargo-audit) are in `ci.yml`. Every
      command passes locally on 2026-10-19. The GitHub run has not been seen
      yet.
- [x] `sandbox-types`: IDs, errors, events, config, and `serde` are present.
      The OpenAPI spec comes from `utoipa` behind the `openapi` feature (ADR 0001).
      `schemars` was removed as unused (PLAN §4.1)
- [x] `sandbox-cas`: `BlobStore` trait, `MemStore`, `DiskStore`, `TieredStore`
- [x] CAS property tests (proptest) in `crates/sandbox-cas/src/disk_store.rs`:
      every blob round-trips, an insertion stores only the chunks around it, and
      a sweep keeps what live blobs name

### Acceptance Criteria
- [ ] CI green on all platforms. The same commands pass locally on macOS
      (2026-10-19). The Linux and GitHub run is not observed
- [x] CAS property tests pass (`cargo test -p sandbox-cas`, 2026-10-19)
- [x] `cargo deny check` passes (`deny.toml`, 2026-10-19)
- [x] `cargo audit --deny warnings` passes (`.cargo/audit.toml`, 2026-10-19)

### Status: **In progress**

---

## M1: Isolate + VFS + Shell Core (Weeks 2-5)

**Goal**: Tier 1 execution with shell, VFS, V8 runtime

### Deliverables
- [x] `sandbox-vfs`: copy-on-write overlay, Merkle commit, symlinks, quotas,
      proptest model checks. ext4 export is not implemented.
- [x] `sandbox-runtime`: `execute_script` runs JavaScript in a fresh V8 isolate with
      `console.log` captured, a wall-clock timeout, an output cap, and a heap cap.
      Module loading is not implemented.
- [x] `sandbox-shell`: parser (quotes, `|`, `&&`, `||`, `;`, `<`, `>`, `>>`),
      interpreter, VFS-backed builtins, variables, globs, and `/bin` WASI tools.
      `case`, `$((...))`, parameter operators, and fd redirection are implemented.
      Host binaries are not. Recursion is bounded at 32 levels, and a limit aborts
      the run (ADR 0018). Fuzz targets exist for the parser, the interpreter, VFS
      paths, and the WASI fs shim (`fuzz/`).
- [x] `sandbox-session`: `IsolateBackend` owns a VFS and runs commands through
      the shell. Snapshot and sleep are not implemented.
- [ ] **Partial.** `sandbox-sdk` (Rust): embedded `exec` and `create_session`
      return `Unsupported`; the in-process path is `sandbox-session` directly
- [x] `cli` (`sandbox run`, `sandbox shell`): execute in an isolate session

### Acceptance Criteria
- [x] `sandbox run "echo hi | cat > /a && cat /a"` works
- [x] `node -e`, `node FILE.js`, and `node FILE.ts` run against VFS (ADR 0024: TypeScript by type stripping; types are not checked)
- [x] Cold start < 5 ms from warm pool in `bench/`: a new Tier 1 session takes a p50 of 833 ns, and a `node` run takes a p50 of 0.41 ms from a warm isolate (5.10 ms cold). The back-to-back tail is the cold cost (ADR 0025)
- [x] 200+ shell conformance cases pass (219 of 219 against GNU bash 5.3.8, ADR 0019)

### Status: **Partial (scaffolding only)**

---

## M2: WASM Tools + Node Compat + Trace (Weeks 6-9)

**Goal**: WASM tooling, Node.js compatibility, tracing

### Deliverables
- [x] `sandbox-wasm`: WASI preview1 command modules run with in-memory stdio, argv,
      fuel, a memory cap, a VFS-backed preopen, and a tool registry.
- [ ] Tools: `jq`, `ripgrep`, `sqlite3`, `git`, `python`
- [ ] Coreutils to 40 commands
- [ ] `node:` shim (`fs`, `path`, `os`, `child_process`, `events`, `stream`, `util`, `buffer`, `url`, `crypto`)
- [x] `sandbox-trace`: trace format, recording from `run` and `shell`, and replay (ADR 0007). The event log in `log.rs` is still unused. Early

### Acceptance Criteria
- [ ] `git clone` (via allowed host), commit, diff works
- [ ] `python -c` and `pip`-free stdlib scripts run
- [x] Trace shows every file change: commands record their file changes (ADR 0011), and writes and deletes through the filesystem API are runs of their own (ADR 0012). Renames and chmod are not separate operations, so a rename shows as a delete and a create
- [ ] 70% pass rate on first agent-transcript corpus

### Status: **Not started**

---

## M3: Server, Policy, Snapshots (Weeks 10-13)

**Goal**: Multi-tenant server with policy, snapshots, scheduling

### Deliverables
- [ ] **Partial.** `sandbox-policy`: capability, limits, and network modules
      exist; no tests
- [ ] **Partial.** `sandbox-snapshot`: early (single module)
- [x] `sandbox-worker`: one child process per session, gRPC over a private Unix socket, with
      crash isolation. Seccomp, cgroups, namespaces, and gRPC are not implemented.
- [ ] **Partial.** `sandbox-server`: session, exec, filesystem, snapshot, and fork routes at `/v1` run on in-memory sessions
      handlers; API keys not implemented (`auth.rs` TODO); no OpenAPI
- [ ] **Partial.** `sandbox-scheduler`: placement, pools, and scheduler modules
      exist; no tests; sleep/wake and metering not verified

### Acceptance Criteria
- [ ] Snapshot/fork under 5 ms
- [ ] 1,000 concurrent sessions on one node in load test
- [x] Sleeping sessions hold no VFS in memory (ADR 0006). With `serve --data-dir`, live and sleeping sessions, their shell state, their traces, their snapshots, and the keys survive a restart (ADRs 0008, 0010).
- [ ] Escape test suite passes
- [ ] TS SDK generated from OpenAPI (current TS SDK is hand-written)

### Status: **Not started (server scaffold only)**

---

## M4: Tier 2 (Weeks 14-19)

**Goal**: Firecracker microVMs, promotion, guest agent

### Deliverables
- [ ] **Partial.** `sandbox-vmm`: driver, jailer, and pool scaffolding
- [ ] **Partial.** `sandbox-guest`: agent and protocol scaffolding
- [ ] Base + browser rootfs images
- [ ] `export_ext4` for promotion
- [ ] `sandbox-promote`: stub. Needs a Linux host with KVM (see plans/PLAN.md M4).
- [ ] VM warm pool, Firecracker snapshots

### Acceptance Criteria
- [ ] `ffmpeg` and headless Chromium run via auto-promotion
- [ ] Promotion under 400 ms
- [ ] VM resume < 150 ms
- [ ] Guest protocol fuzzed for 24h clean

### Status: **Not started**

---

## M5: Production Hardening (Weeks 20-24)

**Goal**: Clustered deployment, security review, operations

### Deliverables
- [ ] Clustered state with Postgres: the metadata store is built and tested against Postgres 18 (ADR 0023). The clustered scheduler is not built, and blobs still need a shared directory
- [ ] Object-store CAS (S3-compatible)
- [ ] CAS GC from live roots
- [ ] Multi-tenant quotas, billing export
- [ ] Dashboards, runbooks
- [ ] External security review

### Acceptance Criteria
- [ ] Chaos suite passes
- [ ] 10,000 idle sessions per host
- [ ] Documented SLOs
- [ ] Security review findings resolved

### Status: **Not started**

---

## M6: Developer Experience (Ongoing)

**Goal**: Polish, examples, integrations

### Deliverables
- [ ] Interactive PTY shell
- [x] `sandbox replay` CLI, with recording through `run --trace` and `shell --trace`
- [ ] VS Code / MCP server
- [ ] Docs site
- [ ] Examples: coding agent, data agent, browser agent
- [ ] Snapshot templates (node-20, python-3.12, etc.)

### Status: **Not started**

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
| M0 | 1 | In progress | GitHub CI run not yet observed; `supply-chain` job added 2026-10-19 |
| M1 | 2-5 | Partial | Shell and isolate `exec` work over the VFS; 219 conformance cases pass against GNU bash (acceptance: 200+ cases). `node -e`, `.ts` against VFS, and the cold-start benchmark are still open |
| M2 | 6-9 | Partial | WASI command modules run over a VFS preopen, with a tool registry; no persistent AOT cache, no preview2 |
| M3 | 10-13 | Partial | Session, exec, fs, snapshot, and fork routes run in memory; API-key auth exists; worker runs one child process per session |
| M4 | 14-19 | Not started | Promotion stub; VMM and guest scaffolding only |
| M5 | 20-24 | Partial | Postgres state store (one node at a time, conditional writes, tested on Postgres 18). Clustered scheduler, placement, and object-store blobs are not built |
| M6 | Ongoing | Not started | - |

---

## Notes

- Work milestone by milestone. Do not start a milestone until the previous one's acceptance criteria pass.
- Start every milestone by writing the public trait/types and tests, then implement.
- Keep crates small and dependencies one-directional.
- Public API changes need an ADR in `docs/adr/`.
- All code must pass `cargo fmt`, `cargo clippy -- -D warnings`, and tests before commit.
