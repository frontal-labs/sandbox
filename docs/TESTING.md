# Testing

This document separates what test infrastructure exists today from the
strategy in PLAN.md section 7. Nothing here means a suite passes. No test run
was performed as part of writing this document.

## What Exists Today

| Area | Location | Kind | Notes |
|------|----------|------|-------|
| `sandbox-cas` | `crates/sandbox-cas/src/mem_store.rs`, `disk_store.rs`, `tiered_store.rs` | Inline `#[test]` unit tests | The only Rust crate with test functions |
| TypeScript SDK | `sdks/typescript/tests/types.test.ts` | vitest | Run with `bun run test` in `sdks/typescript`, or `npm run ts:test` (Bazel) |
| Python SDK | `sdks/python/tests/test_types.py` | pytest | Dev extras in `sdks/python/pyproject.toml` |

Crates and packages with **no test files**: `sandbox-types`, `sandbox-vfs`,
`sandbox-shell`, `sandbox-runtime`, `sandbox-wasm`, `sandbox-session`,
`sandbox-snapshot`, `sandbox-vmm`, `sandbox-guest`, `sandbox-promote`,
`sandbox-policy`, `sandbox-trace`, `sandbox-worker`, `sandbox-scheduler`,
`sandbox-server`, `sdks/rust`, and `cli`.

Declared but unused: `proptest` is a workspace dependency (root `Cargo.toml`),
but no CAS source file uses it. No property tests exist yet.

## Running What Exists

```bash
cargo test --workspace            # Rust; only sandbox-cas has tests
cargo test -p sandbox-cas

cd sdks/typescript && bun run test
cd sdks/python && pytest
```

`cargo test --workspace` compiles all 18 members. Crates without tests report
zero tests.

## CI

`.github/workflows/ci.yml` runs one Rust job: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, and `cargo bench --workspace --no-run`. The commands
run directly, with no Nix wrapper. CI does not run the TypeScript or Python SDK
tests, and does not run `cargo deny` or `cargo audit`. The CI commands have not
been run in this refresh.

## Planned Test Strategy

The sections below describe the target strategy from PLAN.md section 7. Each
item is planned unless it is listed in "What Exists Today".

### Unit Tests (planned conventions)

- Location: inline `#[cfg(test)]` modules in `crates/<crate>/src/`
- Test the public API, not private internals
- Mock external dependencies (CAS, network, time)

### Property Tests (planned)

Targets: `sandbox-vfs` (path normalization, overlay ops, commit and export),
`sandbox-shell` (parser, expansion), `sandbox-cas` (chunking, dedup, refcount
GC), `sandbox-policy` (capability matching, network allowlist). Use `proptest`,
which is already a workspace dependency.

### Conformance Suite (planned)

Shell and coreutils behavior compared against a reference. Target: 200+ shell
cases for M1 (PLAN.md section 9). Directory `tests/conformance/` does not exist
yet.

### Integration Tests (planned)

Server, worker, and SDK in process; multi-session concurrency; snapshot, fork,
and restore; Tier 1 to Tier 2 promotion. Requires the server and worker to
work first, which they do not yet. No `tests/` directory exists.

### Chaos Tests (planned)

Worker crash, corrupt CAS blob, full disk, network partition, clock skew, OOM.
M5 acceptance requires a passing chaos suite.

### Fuzz Tests (planned)

Targets listed for future work: shell parser, VFS path handling, WASI
filesystem shim, guest protocol decoder. No `fuzz/` directories exist yet.

### Agent Transcript Replay (planned)

Replay recorded agent command transcripts against the sandbox. Target: 70%
pass rate on the first corpus (M2). The `sandbox replay` command exists as a
placeholder only.

### Determinism Tests (planned)

Replays of the same trace must produce identical results.

## Benchmarks

- No crate declares a `[[bench]]` target, so `cargo bench` runs nothing today.
- `benchmarks/` (not a workspace member) contains `benches/`, `load/`,
  `common/`, and `baselines/` directories. Whether they build or run was not
  verified.

Targets from PLAN.md section 6, all **not measured**:

| Benchmark | Target |
|-----------|--------|
| VFS read 4 KiB (cached) | < 5 µs p50 |
| Shell parse `echo hi` | < 50 µs |
| CAS put 1 MiB | < 5 ms |
| Snapshot commit | < 5 ms |
| Fork snapshot | < 5 ms |
| Isolate startup (warm) | < 5 ms p50 |
| Promotion (Tier 1 to Tier 2) | < 400 ms p50 |
| VM resume | < 150 ms p50 |

Results are recorded in `docs/decisions/` when a benchmark drives a decision.

## Writing New Tests

Checklist for new work:

- [ ] Unit tests for new public functions
- [ ] Property tests for complex logic (use `proptest`)
- [ ] Conformance cases for new shell features (once the suite exists)
- [ ] Integration test for cross-crate features (once the server works)
- [ ] Benchmark for performance-critical paths
- [ ] Update this document if the test layout changes

Naming (proposed):

```rust
// Unit: test_<function>_<scenario>
#[test]
fn test_commit_returns_new_root_after_write() {}

// Property: prop_<property>
proptest! {
    #[test]
    fn prop_commit_idempotent_on_readonly(/* ... */) {}
}
```
