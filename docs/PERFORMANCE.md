# Performance Targets and Benchmarks

**No performance result in this repository is verified.** There are no recorded benchmark results: `benchmarks/baselines/` is empty, and no document in the repo records measured numbers. The targets below are design goals from PLAN.md section 6. Do not read them as measured results or as claims that the system meets them. The benchmark suite compiles against crates that are mostly stubs, so its output does not yet measure the target paths.

## Targets

Source: PLAN.md section 6. Milestone acceptance criteria are in PLAN.md section 9.

| Metric | Target | Benchmark that would measure it | Status |
|--------|--------|---------------------------------|--------|
| Isolate session from warm pool | < 5 ms p50, < 15 ms p99 | `runtime_bench`: `cold_isolate_console_log` | Bench measures a cold isolate per run. No pool exists, so the warm-pool target is not measured. |
| Restore from snapshot (Tier 1) | < 25 ms p50 | none | Restore works in memory. Latency is not measured. |
| `echo hi` end-to-end (local) | < 10 ms | `session_bench`: `session_exec/echo_argv` | Bench exists and runs real code. Not measured in this repo. |
| VFS read 4 KiB (cached) | < 5 µs | `vfs_bench`: `read_4kb_cached` | Bench exists. Not measured. |
| Fork a snapshot | < 5 ms | `session_bench` | Commented out. Not measured. |
| Tier 2 resume from snapshot | < 150 ms p50 | none | Tier 2 is not implemented. |
| Promotion (Tier 1 to Tier 2) | < 400 ms (M4 acceptance) | none | Promotion is not implemented. |
| Idle sessions per 64 GB host | 10,000+ | `load` scenario `concurrent_sessions` | Scenario uses sleeps and is not a capacity test. |
| Idle memory per sleeping session | 0 (on disk only) | none | Sleep releases the live session and keeps its root in server memory (ADR 0006). Nothing is written to disk, and no memory is measured. |

Earlier drafts also listed p99 and capacity numbers that do not appear in PLAN.md section 6. They have been removed.

## Benchmark suite

The benchmark code lives in `benchmarks/`. The layout is described in [benchmarks/README.md](../benchmarks/README.md).

### Micro-benchmarks (Criterion)

Six Criterion suites are in `benchmarks/benches/`: `vfs_bench`, `cas_bench`, `shell_bench`, `session_bench`, `runtime_bench`, and `wasm_bench`. All six run real code. `promotion_bench` was removed because it measured no-ops, so there is no promotion benchmark until promotion exists.

`benchmarks` is a root workspace member, so `cargo bench --workspace --no-run` compiles these suites, as CI does. The commands below are not verified in this refresh:

```bash
# Intended invocation, run from the benchmarks directory (not verified)
cd benchmarks
cargo bench --bench vfs_bench
cargo bench --bench cas_bench
```

Do not run these in CI assuming they produce a baseline. See Regression detection.

### Load driver

`benchmarks/load/` builds the `sandbox-load` binary (`benchmarks/load/src/main.rs`). It has three subcommands: `run`, `compare`, and `list`. Scenarios are `concurrent_sessions`, `exec_throughput`, `snapshot_fork`, and `mixed_workload`.

The scenarios use the embedded SDK, which calls into the unimplemented session backend. The `--server-url` flag is parsed and stored in the scenario config, but no scenario in the repository uses it, and no server exists to target. Treat load-driver output as a harness check, not as a measurement.

The k6 scripts described in earlier drafts (`bench/load/*.js`) do not exist.

## Running the suite

```bash
# Not verified: benchmarks/load is not a root workspace member either
cd benchmarks/load
cargo build --release
./target/release/sandbox-load list
./target/release/sandbox-load run --scenario exec_throughput --concurrency 50 --duration 10
```

## Regression detection

`.github/workflows/benchmarks.yml` runs on push, pull request, and nightly. The intended check fails the job when a benchmark's p99 rises more than 10% or a load test's throughput drops more than 10% against `benchmarks/baselines/`.

Current state of that workflow:

- `benchmarks/baselines/` is empty, so the comparison logic skips every suite with "No baseline" and passes. A pass means nothing.
- The micro-benchmark step runs the bench targets from the repository root with `cargo bench --bench <name>`, which will not find them because the suites are outside the workspace. Verify before relying on it.
- The `bench-report` job prints a hard-coded "Passed" for each suite, not the real job results.
- Baseline updates on master pushes copy results into `benchmarks/baselines/` inside the CI runner. Nothing commits them, so the copies are discarded when the job ends.

Until these are fixed, the workflow does not detect regressions.

## Profiling

Standard tools apply once a benchmark runs: `cargo flamegraph`, `perf record`, and `heaptrack`. The commands in earlier drafts used package and bench names that do not exist. Use the bench names listed above.

## Planned optimizations

These are design ideas from PLAN.md and earlier drafts. None are implemented, and none have measured effects.

- **VFS**: mmap for CAS reads above 64 KiB; batched commits that rehash only dirty paths (PLAN.md section 4.3).
- **CAS**: FastCDC chunking for files above 64 KiB (PLAN.md section 4.2). The code has a `chunk_if_large` flag and a `put_chunked` path, but no FastCDC implementation. Compression at zstd level 3 is in `crates/sandbox-cas/src/disk_store.rs`.
- **Isolate**: startup snapshot pooled by bootstrap version; context recycling instead of isolate recreation (PLAN.md section 4.5).
- **Promotion**: export cache keyed by VFS root hash; warm VM pool per image (PLAN.md sections 4.10 and 4.11).

## Performance testing checklist

Pre-release checks that have not been done:

- [ ] Each target above measured on recorded hardware, with results committed to `docs/decisions/` (PLAN.md section 11, item 8)
- [ ] Criterion suites run from a path the workspace can build
- [ ] Baselines recorded in `benchmarks/baselines/`
- [ ] Regression check verified to fail on a known regression
- [ ] Load test at the target concurrency, with a real server and worker
