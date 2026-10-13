# Benchmarking System Plan for Frontal Sandbox

## Overview

Based on `PLAN.md` Section 6 and the repository layout, this plan creates a proper benchmarking system under `bench/` (referred to as `/benchmarks` in the request) that includes:

1. **Micro-benchmarks** using `criterion` for VFS, shell parser, CAS, runtime ops, WASM tools
2. **End-to-end load tests** using a Rust driver for session creation, exec, snapshot, fork
3. **CI integration** with regression detection (>10% failure threshold)
4. **Performance target validation** against the targets in PLAN.md Section 6

## Current State

- No `bench/` directory exists
- `criterion` is already in workspace dependencies (v0.5)
- CI has a "Bench compile check" but no actual benchmark runs
- All crates are in early development (M0-M1 stage per PLAN.md)

## Benchmark Categories & Targets (from PLAN.md Section 6)

| Metric | Target |
|--------|--------|
| Isolate session from warm pool | < 5 ms p50, < 15 ms p99 |
| Restore from snapshot (Tier 1) | < 25 ms p50 |
| `echo hi` end-to-end (local) | < 10 ms |
| VFS read 4 KiB (cached) | < 5 µs |
| Fork a snapshot | < 5 ms |
| Tier 2 resume from snapshot | < 150 ms p50 |
| Idle sessions per 64 GB host | 10,000+ |
| Idle memory per sleeping session | 0 (on disk only) |

## Implementation Plan

### Phase 1: Benchmark Infrastructure Setup

**Task 1.1: Create `bench/` directory structure**
```
bench/
├── Cargo.toml                 # Benchmark workspace member
├── benches/
│   ├── vfs_bench.rs           # VFS micro-benchmarks
│   ├── cas_bench.rs           # CAS micro-benchmarks
│   ├── shell_bench.rs         # Shell parser/interpreter benchmarks
│   ├── runtime_bench.rs       # V8 isolate startup/ops benchmarks
│   ├── wasm_bench.rs          # WASM tool startup/execution benchmarks
│   ├── session_bench.rs       # Session create/exec/snapshot/fork benchmarks
│   └── promotion_bench.rs     # Tier 1 -> Tier 2 promotion benchmarks
├── load/
│   ├── Cargo.toml             # Load test binary
│   └── src/
│       ├── main.rs            # Load test driver
│       ├── scenarios/         # Load test scenarios
│       │   ├── concurrent_sessions.rs
│       │   ├── exec_throughput.rs
│       │   ├── snapshot_fork.rs
│       │   └── mixed_workload.rs
│       └── reporter.rs        # Results reporting
├── common/
│   └── mod.rs                 # Shared benchmark utilities
└── README.md                  # Benchmark documentation
```

**Task 1.2: Add `bench` to workspace Cargo.toml**
- Add `bench` to workspace members
- Configure `criterion` as dev-dependency for benches

**Task 1.3: Create benchmark utilities (`bench/common/mod.rs`)**
- Warm pool simulation helpers
- Session factory for consistent test setup
- Metrics collection (p50, p99, p999, throughput)
- Output formatting for CI consumption (JSON for regression comparison)

### Phase 2: Micro-Benchmarks (criterion)

**Task 2.1: VFS Benchmarks (`benches/vfs_bench.rs`)**
- `vfs_read_4kb_cached` - Target: < 5 µs
- `vfs_write_4kb` - Write throughput
- `vfs_commit_incremental` - Commit dirty paths only
- `vfs_readdir` - Directory listing
- `vfs_overlay_operations` - Copy-on-write overlay reads/writes

**Task 2.2: CAS Benchmarks (`benches/cas_bench.rs`)**
- `cas_put_get_small` - Small blob (< 64 KiB) put/get
- `cas_put_get_large` - Large blob with FastCDC chunking
- `cas_has` - Existence check performance
- `cas_tiered_store` - Local cache + remote fallback

**Task 2.3: Shell Benchmarks (`benches/shell_bench.rs`)**
- `shell_parse_simple` - Simple command parsing
- `shell_parse_complex` - Pipes, redirects, subshells
- `shell_exec_builtin` - Builtin execution (cd, echo, etc.)
- `shell_exec_pipeline` - Multi-command pipelines
- `shell_variable_expansion` - Parameter expansion variants

**Task 2.4: Runtime Benchmarks (`benches/runtime_bench.rs`)**
- `isolate_startup_warm_pool` - Target: < 5 ms p50
- `isolate_startup_snapshot` - Target: < 25 ms p50
- `js_exec_simple` - Simple JS execution
- `js_fs_ops` - V8 -> VFS ops (op_fs_*)
- `js_fetch` - HTTP fetch ops

**Task 2.5: WASM Benchmarks (`benches/wasm_bench.rs`)**
- `wasm_module_load` - AOT module load time
- `wasm_tool_startup` - Tool (jq, ripgrep) cold start
- `wasm_tool_execution` - Tool execution throughput

**Task 2.6: Session Benchmarks (`benches/session_bench.rs`)**
- `session_create_isolate` - IsolateBackend creation
- `session_exec_simple` - Target: < 10 ms for `echo hi`
- `session_snapshot` - Target: < 5 ms
- `session_fork` - Target: < 5 ms
- `session_restore` - Target: < 25 ms p50

**Task 2.7: Promotion Benchmarks (`benches/promotion_bench.rs`)**
- `promote_tier1_to_tier2` - Full promotion path
- `export_ext4` - VFS to ext4 image conversion
- `vm_attach_image` - Firecracker image attachment
- `vm_resume_snapshot` - Target: < 150 ms p50

### Phase 3: End-to-End Load Tests

**Task 3.1: Load Test Binary (`load/Cargo.toml` + `load/src/main.rs`)**
- CLI with subcommands: `run`, `report`, `compare`
- Configurable: concurrency, duration, ramp-up, scenarios
- Outputs: JSON summary, latency histograms, throughput

**Task 3.2: Load Test Scenarios**
- `concurrent_sessions` - 1000+ concurrent idle sessions (memory check)
- `exec_throughput` - Max exec/sec with varying complexity
- `snapshot_fork` - Snapshot/fork under load
- `mixed_workload` - Realistic agent workload (git, npm, test)

**Task 3.3: Metrics Collection**
- Latency percentiles (p50, p90, p99, p999)
- Throughput (ops/sec)
- Error rates
- Resource usage (memory, CPU via `/proc` or `sysinfo`)

### Phase 4: CI Integration & Regression Detection

**Task 4.1: GitHub Actions Workflow (`.github/workflows/benchmarks.yml`)**
- Runs on `push` to master and scheduled (nightly)
- Steps:
  1. Compile benchmarks (`cargo bench --no-run`)
  2. Run micro-benchmarks (`cargo bench --bench <name>`)
  3. Run load tests (short duration for CI)
  4. Compare against baseline (stored in `bench/baselines/`)
  5. Fail if any metric regresses >10%

**Task 4.2: Baseline Management**
- Store baselines in `bench/baselines/<crate>_<bench>.json`
- Update baselines on successful master runs (optional: manual approval)
- Track history for trend analysis

**Task 4.3: Regression Report**
- Generate markdown summary for PR comments
- Include: metric, baseline, current, delta, pass/fail

### Phase 5: Documentation & Developer Experience

**Task 5.1: `bench/README.md`**
- How to run benchmarks locally
- How to interpret results
- How to add new benchmarks
- Baseline update process

**Task 5.2: CLI Integration (`cli/src/main.rs`)**
- Add `sandbox bench` command
- Subcommands: `micro`, `load`, `compare`
- Delegates to `bench` binaries

## Dependencies Needed

**Already in workspace:**
- `criterion` (v0.5) - micro-benchmarks
- `tokio`, `tracing` - async/runtime
- `serde`, `serde_json` - serialization

**To add to workspace dependencies:**
- `sysinfo` - System resource monitoring (load tests)
- `histogram` or `hdrhistogram` - Latency histograms
- `clap` - Load test CLI (already in workspace)
- `indicatif` - Progress bars (optional, for load tests)
- `csv` - CSV export for analysis

## Acceptance Criteria

1. **Micro-benchmarks run**: `cargo bench --workspace` executes all benches without errors
2. **Targets validated**: Each benchmark documents its target from PLAN.md Section 6
3. **Load tests execute**: `cargo run --release -p sandbox-load -- <scenario>` runs successfully
4. **CI integration**: Nightly workflow runs, compares against baselines, fails on >10% regression
5. **Baseline storage**: Baselines committed to `bench/baselines/`
6. **Documentation**: `bench/README.md` explains usage

## Open Questions

1. **Baseline storage**: Commit baselines to git or store as CI artifacts? (Recommend: git for reproducibility)
2. **Load test duration**: Short CI run (30s) vs full validation (5min)? (Recommend: 30s for CI, manual for full)
3. **Hardware consistency**: CI runners vary. Use relative comparison only? (Recommend: relative to baseline on same runner type)
4. **Tier 2 benchmarks**: Require KVM/Firecracker. Run only on self-hosted runners? (Recommend: separate workflow, gated by feature flag)

## Implementation Order

1. Phase 1 (Infrastructure) - Foundation for all benchmarks
2. Phase 2 (Micro-benchmarks) - Start with VFS, CAS, Shell (available now)
3. Phase 3 (Load tests) - Once session/exec work end-to-end
4. Phase 4 (CI) - After benchmarks produce stable results
5. Phase 5 (Docs/CLI) - Polish

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Benchmarks flaky on CI | Use statistical rigor (criterion defaults), multiple runs |
| No Tier 2 in CI | Gate behind feature flag, run on self-hosted KVM runners |
| Baseline drift | Manual approval for baseline updates, track in CHANGELOG |
| Micro-benchmarks don't reflect real perf | Complement with load tests, validate with profiling |

## Next Steps

1. Review this plan and confirm approach
2. Begin Phase 1 implementation (infrastructure + first micro-benchmarks)
3. Iterate as crates mature through milestones M1-M4