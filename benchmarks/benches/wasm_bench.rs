//! WASI command benchmarks. Each run compiles and instantiates the module, so
//! these measure load plus execution. Precompiled (.cwasm) loading is not
//! implemented yet.

use criterion::{criterion_group, criterion_main, Criterion};
use sandbox_wasm::{run_module, WasiLimits};
use tokio::runtime::Runtime;

/// Writes "hi\n" to stdout and exits with status 0.
const HELLO_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "hi\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 3))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))
    (call $proc_exit (i32.const 0))))
"#;

fn wasm_run_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("wasm_run");

    group.bench_function("hello_module", |b| {
        b.to_async(&rt).iter(|| async {
            run_module(
                HELLO_WAT.as_bytes().to_vec(),
                vec!["hello".to_string()],
                Vec::new(),
                WasiLimits::default(),
            )
            .await
            .unwrap()
        });
    });

    group.finish();
}

criterion_group!(benches, wasm_run_benchmarks);
criterion_main!(benches);
