# Frontal Sandbox Benchmarks

This directory holds the Criterion micro-benchmarks and the `sandbox-load` end-to-end load driver.

**Status:** the benchmarks are scaffolds. Most of the code they call is stubbed (see [ARCHITECTURE.md](../docs/ARCHITECTURE.md)), and no benchmark result has been recorded. `benchmarks/baselines/` is empty. Do not cite any number from this directory as a measurement. Targets are in [PERFORMANCE.md](../docs/PERFORMANCE.md) and the project specification section 6.

## Layout

```
benchmarks/
├── Cargo.toml           # package "sandbox-bench"; NOT a member of the root workspace
├── BUILD.bazel          # Bazel targets for the suites and common library
├── benches/             # Criterion suites (see table below)
├── common/              # shared helpers: mod.rs plus one helper file per area
├── load/                # package "sandbox-load" (binary "sandbox-load")
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs      # CLI: run, compare, list
│       ├── scenarios.rs # ScenarioType enum and get_scenario()
│       ├── scenarios/   # concurrent_sessions, exec_throughput, snapshot_fork, mixed_workload
│       └── reporter.rs  # report generation
├── baselines/           # empty; CI reads and writes baseline JSON here
└── README.md
```

### Micro-benchmark suites

| File | Criterion group(s) | Status |
|------|--------------------|--------|
| `benches/vfs_bench.rs` | `vfs`, `vfs_overlay` | Includes `read_4kb_cached` (target in the project specification section 6) |
| `benches/cas_bench.rs` | `cas_mem`, `cas_disk` | Mem and disk store cases |
| `benches/shell_bench.rs` | `shell_parser`, `shell_interpreter` | Builtin and pipeline cases; the parser is a stub around `brush-parser` |
| `benches/runtime_bench.rs` | `runtime_startup`, `runtime_execution`, `runtime_ops` | Calls into `sandbox-runtime`, whose `execute_script` is unimplemented |
| `benches/wasm_bench.rs` | `wasm_load`, `wasm_tools` | WASM execution is unimplemented |
| `benches/session_bench.rs` | none | Every group is commented out; the file measures nothing |
| `benches/promotion_bench.rs` | `promotion`, `vm_resume` | Promotion and VM resume are unimplemented |

### Load scenarios

| Scenario | Source | Notes |
|----------|--------|-------|
| `concurrent_sessions` | `load/src/scenarios/concurrent_sessions.rs` | Uses sleeps; not a capacity test |
| `exec_throughput` | `load/src/scenarios/exec_throughput.rs` | Runs through the embedded SDK |
| `snapshot_fork` | `load/src/scenarios/snapshot_fork.rs` | Includes a fixed 5 ms sleep per iteration |
| `mixed_workload` | `load/src/scenarios/mixed_workload.rs` | Only in the nightly CI job |

The embedded SDK calls into the unimplemented session backend, so these scenarios do not yet exercise real sandbox work.

## Running

These commands are not verified in this refresh. `benchmarks/` is not a member of the root Cargo workspace, so `cargo bench --workspace` from the repository root does not run these suites, and `cargo build -p sandbox-load` from the root does not find the package. Run them from the `benchmarks/` directory:

```bash
cd benchmarks

# Criterion suites
cargo bench --bench vfs_bench
cargo bench --bench cas_bench

# Load driver
cargo build --release --manifest-path load/Cargo.toml
./load/target/release/sandbox-load list
./load/target/release/sandbox-load run --scenario concurrent_sessions --concurrency 100 --duration 10
```

The `run` subcommand takes `--scenario`, `--concurrency`, `--duration`, `--ramp-up`, `--format`, `--output`, and `--server-url`. The `compare` subcommand takes a current file, a baseline file, and a threshold. The `--server-url` value is stored but no scenario uses it yet.

## CI

`.github/workflows/benchmarks.yml` runs on push to master, on pull requests to master, and nightly. It:

1. Compiles the benchmarks with `cargo bench --workspace --no-run`. This does not include `benchmarks/`, so the step does not compile these suites.
2. Runs each Criterion suite with `cargo bench --bench <name> -- --output-format=json`, from the repository root. This will fail because the suites are not workspace targets. Verify before relying on it.
3. Compares p99 values against `benchmarks/baselines/<name>.json` with a 10% threshold. The baselines are empty, so every suite is skipped and the check passes.
4. Runs short load tests (10 s) and nightly full load tests (60 to 120 s). The workflow builds these with `cargo build --release -p sandbox-load`, which is not a root workspace package.
5. Writes a summary. The summary's "Passed" marks are hard-coded, not taken from job results.

On master pushes the workflow copies results into `benchmarks/baselines/` on the runner. Nothing commits them.

## Baselines

`baselines/` is empty. To record a baseline after a real measurement exists, run the suite, save its output, and commit the JSON file. Note the hardware, the commit, and the toolchain in the commit message or in `docs/decisions/`. The CI regression check expects a JSON array of objects with `name`, `mean`, and `p99` fields. Criterion does not emit that shape by default, and `--output-format=json` is not a documented Criterion flag, so verify the format with a real run before trusting the check.

## Adding a benchmark

### Micro-benchmark

1. Add `benches/<name>.rs` with a `criterion_group!` and `criterion_main!`.
2. Add a `[[bench]]` entry with `harness = false` to `benchmarks/Cargo.toml`.
3. Add a `rust_binary` target to `BUILD.bazel`, following the existing entries.
4. Cite the matching target in the project specification section 6 in a comment. Do not add numbers you have not measured.

### Load scenario

1. Add `load/src/scenarios/<name>.rs` with a struct and `impl` block, following the existing files.
2. Add a variant to `ScenarioType` and a match arm in `ScenarioType::from_str` in `load/src/scenarios.rs`, and wire it into `get_scenario()`.
3. Document the scenario in the table above.

## Related documentation

- [PERFORMANCE.md](../docs/PERFORMANCE.md): targets and status
- [SECURITY_MODEL.md](../docs/SECURITY_MODEL.md): security status
