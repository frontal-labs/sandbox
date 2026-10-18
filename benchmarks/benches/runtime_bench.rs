//! V8 script benchmarks. Each run creates a fresh isolate (no pool yet), so
//! these measure cold start plus execution. plans/PLAN.md section 6 targets a warm
//! pool at < 5 ms p50; that path does not exist yet.

use criterion::{criterion_group, criterion_main, Criterion};
use sandbox_runtime::{execute_script, ScriptLimits};
use tokio::runtime::Runtime;

fn runtime_script_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("runtime_script");

    group.bench_function("cold_isolate_console_log", |b| {
        b.to_async(&rt).iter(|| async {
            execute_script("console.log(1)", ScriptLimits::default())
                .await
                .unwrap()
        });
    });

    group.bench_function("loop_10k_iterations", |b| {
        let code = "let s = 0; for (let i = 0; i < 10000; i++) s += i; console.log(s);";
        b.to_async(&rt)
            .iter(|| async { execute_script(code, ScriptLimits::default()).await.unwrap() });
    });

    group.finish();
}

criterion_group!(benches, runtime_script_benchmarks);
criterion_main!(benches);
