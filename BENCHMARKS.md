# Frontal Benchmarks

This document records the benchmark results for the Frontal Sandbox system. All benchmarks were run on the repository commit at the time of measurement. Hardware: Apple M-series, macOS, Rust 1.95, toolchain from `rust-toolchain.toml`.

---

## Micro-Benchmarks (Criterion)

### VFS (Virtual File System)

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| vfs/create | 74.59 | 75.45 |
| vfs/stat_root | 132.88 | 138.34 |
| vfs/mkdir | 595.89 | 663.26 |
| vfs/read_4kb_overlay | 261.20 | 277.17 |
| vfs/create_and_write | 1,126.47 | 1,167.38 |
| vfs/commit_small | 3,409.39 | 3,631.67 |
| vfs/readdir_100 | 13,158.70 | 14,330.21 |
| vfs/commit_10_files | 42,251.66 | 46,694.49 |
| vfs_overlay/rename_file | 2,534.28 | 2,640.60 |
| vfs_overlay/symlink | 585.21 | 686.22 |
| vfs_overlay/write_new_file | 20,235.54 | 37,975.59 |

*Source: `benchmarks/baselines/vfs.json`*

---

### CAS (Content Addressable Storage)

#### In-Memory Store

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| cas_mem/has | 56.37 | 56.95 |
| cas_mem/get_small | 62.88 | 64.33 |
| cas_mem/get_1mb | 62.83 | 63.35 |
| cas_mem/list_empty | 37.74 | 38.09 |
| cas_mem/list_100 | 235.56 | 239.16 |
| cas_mem/delete | 52.22 | 67.47 |
| cas_mem/put_small | 1,249.69 | 1,378.95 |
| cas_mem/put_64kb | 28,188.85 | 28,873.70 |
| cas_mem/put_1mb | 280,010.26 | 305,879.75 |

#### Disk Store

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| cas_disk/has | 1,933.43 | 1,953.82 |
| cas_disk/put_small | 3,092.62 | 3,358.50 |
| cas_disk/put_64kb | 17,481.86 | 18,118.96 |
| cas_disk/get_small | 71,999.68 | 82,608.10 |
| cas_disk/put_1mb | 255,459.81 | 268,167.66 |
| cas_disk/get_1mb | 897,223.33 | 933,676.14 |

#### Tiered Store (Memory + Disk)

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| cas_tiered/put_small | 3,134.50 | 3,422.14 |
| cas_tiered/get_local | 72,244.80 | 85,082.52 |
| cas_tiered/put_1mb | 254,042.08 | 261,258.33 |

*Source: `benchmarks/baselines/cas.json`*

---

### Shell

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| shell_parser/simple | 570.23 | 654.37 |
| shell_parser/pipeline | 1,194.24 | 1,263.97 |
| shell_parser/operators_and_redirects | 2,698.21 | 2,823.48 |
| shell_exec/echo | 1,690.25 | 1,871.76 |
| shell_exec/pipe_and_redirect | 7,378.38 | 7,933.74 |

*Source: `benchmarks/baselines/shell.json`*

---

### Runtime (JavaScript/Deno)

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| runtime_script/cold_isolate_console_log | 4,390,874.63 | 5,500,694.75 |
| runtime_script/loop_10k_iterations | 4,494,042.77 | 5,254,368.40 |

*Source: `benchmarks/baselines/runtime.json`*

---

### WASM

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| wasm_run/hello_module | 310,509.99 | 363,512.97 |

*Source: `benchmarks/baselines/wasm.json`*

---

### Session

| Benchmark | Mean (ns) | P99 (ns) |
|-----------|-----------|----------|
| session_create/isolate_cold | 348.22 | 353.27 |
| session_fs/read_4kb | 2,180.81 | 2,228.58 |
| session_exec/echo_argv | 1,116.68 | 1,222.26 |
| session_exec/run_script_pipeline | 8,904.87 | 9,169.43 |

*Source: `benchmarks/baselines/session.json`*

---

## Load Tests (End-to-End)

### Concurrent Sessions

- **Scenario:** `concurrent_sessions`
- **Duration:** 10.28s
- **Concurrency:** 100
- **Total Iterations:** 10,082,761
- **Total Errors:** 0
- **Throughput:** 980,347 ops/sec

| Metric | Value |
|--------|-------|
| Latency Min | 1,703 ns |
| Latency Mean | 89,174.72 ns |
| Latency P50 | 3,396 ns |
| Latency P90 | 3,617 ns |
| Latency P95 | 3,767 ns |
| Latency P99 | 18,825 ns |
| Latency Max | 69,828,399 ns |
| Std Dev | 1,241,852.61 ns |

*Source: `benchmarks/baselines/load_concurrent.json` (timestamp: 2026-11-03T09:00:03Z)*

---

### Execution Throughput

- **Scenario:** `exec_throughput`
- **Duration:** 10.29s
- **Concurrency:** 50
- **Total Iterations:** 9,812,054
- **Total Errors:** 0
- **Throughput:** 953,959 ops/sec

| Metric | Value |
|--------|-------|
| Latency Min | 972 ns |
| Latency Mean | 45,026.30 ns |
| Latency P50 | 3,236 ns |
| Latency P90 | 3,837 ns |
| Latency P95 | 3,958 ns |
| Latency P99 | 18,826 ns |
| Latency Max | 27,841,073 ns |
| Std Dev | 611,682.61 ns |

*Source: `benchmarks/baselines/load_throughput.json` (timestamp: 2026-11-03T09:00:13Z)*

---

### Snapshot Fork

- **Scenario:** `snapshot_fork`
- **Duration:** 10.03s
- **Concurrency:** 20
- **Total Iterations:** 832,882
- **Total Errors:** 0
- **Throughput:** 83,071 ops/sec

| Metric | Value |
|--------|-------|
| Latency Min | 16,712 ns |
| Latency Mean | 21,721.76 ns |
| Latency P50 | 21,260 ns |
| Latency P90 | 22,011 ns |
| Latency P95 | 22,742 ns |
| Latency P99 | 35,676 ns |
| Latency Max | 708,910 ns |
| Std Dev | 2,955.81 ns |

*Source: `benchmarks/baselines/load_snapshot.json` (timestamp: 2026-11-03T09:00:24Z)*

---

## Summary

| Category | Key Metric |
|----------|------------|
| VFS create | ~75 ns |
| VFS read 4KB (overlay) | ~261 ns |
| CAS mem get (small) | ~63 ns |
| CAS disk get (1MB) | ~897 µs |
| Shell parse (simple) | ~570 ns |
| Shell exec (echo) | ~1.7 µs |
| Runtime cold isolate | ~4.4 ms |
| WASM hello module | ~310 µs |
| Session create (cold) | ~348 ns |
| Session exec (echo) | ~1.1 µs |
| Concurrent sessions throughput | ~980k ops/sec |
| Exec throughput | ~954k ops/sec |
| Snapshot fork throughput | ~83k ops/sec |

---

*Generated from baseline files in `benchmarks/baselines/`. Run `cargo bench` in `benchmarks/` to regenerate.*