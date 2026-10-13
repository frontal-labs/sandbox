# Milestones

Based on [PLAN.md](../PLAN.md) section 9. A box is ticked only when the code
clearly implements the item and a test exists for it. Items that are scaffolded
but not working, or implemented without tests, stay unticked and are marked
**Partial**. Acceptance criteria are not ticked unless they were demonstrated,
and none has been demonstrated yet.

Last reviewed against the source tree on 2026-10-13.

## M0: Foundations (Week 1)

**Goal**: Cargo workspace, CI, `sandbox-types`, `sandbox-cas` (Mem + Disk)

### Deliverables
- [x] Cargo workspace with all 18 crate members present (`Cargo.toml`)
- [ ] **Partial.** CI pipeline: fmt, clippy `-D warnings`, test present in
      `ci.yml` and run without Nix, but `deny` and `audit` are absent and the
      pipeline has not been verified to pass
- [ ] **Partial.** `sandbox-types`: IDs, errors, events, config, and `serde`
      are present. `schemars` is a dependency, but no type derives `JsonSchema`
- [x] `sandbox-cas`: `BlobStore` trait, `MemStore`, `DiskStore`, `TieredStore`
- [ ] CAS property tests (proptest). Not present; the CAS tests are unit tests

### Acceptance Criteria
- [ ] CI green on all platforms. Not verified; CI depends on the removed flake
- [ ] CAS property tests pass. No property tests exist
- [ ] `cargo deny check` passes. `deny.toml` does not exist
- [ ] `cargo audit` clean. Not configured in CI

### Status: **In progress**

---

## M1: Isolate + VFS + Shell Core (Weeks 2-5)

**Goal**: Tier 1 execution with shell, VFS, V8 runtime

### Deliverables
- [ ] **Partial.** `sandbox-vfs`: layer, node, quota, and ops modules exist.
      Overlay and commit are not verified; ext4 export is not present; no tests
- [ ] **Partial.** `sandbox-runtime`: `deno_core` host set up. Script execution
      is a TODO that returns an error
- [ ] **Partial.** `sandbox-shell`: parser is a stub around `brush-parser`;
      interpreter and builtins are minimal
- [ ] **Not started.** `sandbox-session`: `IsolateBackend` is `unimplemented!()`
- [ ] **Partial.** `sandbox-sdk` (Rust): embedded module exists, but client
      methods are stubs and no isolate backend backs it
- [ ] **Partial.** `cli` (`sandbox run`): prints a message; execution is a TODO

### Acceptance Criteria
- [ ] `sandbox run "echo hi | cat > /a && cat /a"` works
- [ ] `node -e` and `.ts` script run against VFS
- [ ] Cold start < 5 ms from warm pool in `bench/`
- [ ] 200+ shell conformance cases pass

### Status: **Partial (scaffolding only)**

---

## M2: WASM Tools + Node Compat + Trace (Weeks 6-9)

**Goal**: WASM tooling, Node.js compatibility, tracing

### Deliverables
- [ ] **Partial.** `sandbox-wasm`: crate present; WASI execution is a TODO
- [ ] Tools: `jq`, `ripgrep`, `sqlite3`, `git`, `python`
- [ ] Coreutils to 40 commands
- [ ] `node:` shim (`fs`, `path`, `os`, `child_process`, `events`, `stream`, `util`, `buffer`, `url`, `crypto`)
- [ ] **Partial.** `sandbox-trace`: `log.rs` and `replay.rs` exist; early

### Acceptance Criteria
- [ ] `git clone` (via allowed host), commit, diff works
- [ ] `python -c` and `pip`-free stdlib scripts run
- [ ] Trace shows every file change
- [ ] 70% pass rate on first agent-transcript corpus

### Status: **Not started**

---

## M3: Server, Policy, Snapshots (Weeks 10-13)

**Goal**: Multi-tenant server with policy, snapshots, scheduling

### Deliverables
- [ ] **Partial.** `sandbox-policy`: capability, limits, and network modules
      exist; no tests
- [ ] **Partial.** `sandbox-snapshot`: early (single module)
- [ ] `sandbox-worker`: stub (seccomp, cgroups, gRPC not implemented)
- [ ] **Partial.** `sandbox-server`: routes wired at `/v1` with placeholder
      handlers; API keys not implemented (`auth.rs` TODO); no OpenAPI
- [ ] **Partial.** `sandbox-scheduler`: placement, pools, and scheduler modules
      exist; no tests; sleep/wake and metering not verified

### Acceptance Criteria
- [ ] Snapshot/fork under 5 ms
- [ ] 1,000 concurrent sessions on one node in load test
- [ ] Sleeping sessions use no memory
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
- [ ] **Partial.** `sandbox-promote`: stub
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
- [ ] Clustered scheduler with Postgres
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
- [ ] `sandbox replay` CLI (placeholder subcommand only)
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
| M0 | 1 | In progress | CI depends on removed Nix flake; no `deny.toml`; no proptest suite |
| M1 | 2-5 | Partial (scaffolding) | `IsolateBackend` unimplemented; shell parser stub; runtime script execution TODO |
| M2 | 6-9 | Not started | WASI execution TODO |
| M3 | 10-13 | Not started | Server handlers are placeholders; auth TODO; worker stub |
| M4 | 14-19 | Not started | Promotion stub; VMM and guest scaffolding only |
| M5 | 20-24 | Not started | - |
| M6 | Ongoing | Not started | - |

---

## Notes

- Work milestone by milestone. Do not start a milestone until the previous one's acceptance criteria pass.
- Start every milestone by writing the public trait/types and tests, then implement.
- Keep crates small and dependencies one-directional.
- Public API changes need an ADR in `docs/adr/`.
- All code must pass `cargo fmt`, `cargo clippy -- -D warnings`, and tests before commit.
