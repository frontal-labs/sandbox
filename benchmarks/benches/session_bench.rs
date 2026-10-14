//! Session micro-benchmarks for the isolate tier.
//!
//! PLAN.md section 6 targets: isolate session from a warm pool < 5 ms p50.
//! This measures cold creation, since there is no pool yet.

use criterion::{criterion_group, criterion_main, Criterion};
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{ExecRequest, SessionConfig};
use sandbox_vfs::OpenFlags;
use std::path::Path;
use tokio::runtime::Runtime;

fn session_create_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("session_create");
    group.bench_function("isolate_cold", |b| {
        b.to_async(&rt)
            .iter(|| async { IsolateBackend::new(SessionConfig::default()).await.unwrap() });
    });
    group.finish();
}

fn session_exec_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let session = rt
        .block_on(IsolateBackend::new(SessionConfig::default()))
        .unwrap();
    let mut group = c.benchmark_group("session_exec");

    group.bench_function("echo_argv", |b| {
        b.to_async(&rt).iter(|| async {
            let req = ExecRequest {
                argv: vec!["echo".to_string(), "hi".to_string()],
                ..ExecRequest::default()
            };
            session.exec(req).await.unwrap()
        });
    });

    group.bench_function("run_script_pipeline", |b| {
        b.to_async(&rt).iter(|| async {
            session
                .run_script("echo hi | cat > /bench && cat /bench")
                .await
                .unwrap()
        });
    });

    group.finish();
}

fn session_fs_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let session = rt
        .block_on(IsolateBackend::new(SessionConfig::default()))
        .unwrap();
    let write_flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    rt.block_on(async {
        let h = session
            .fs()
            .open(Path::new("/data"), write_flags)
            .await
            .unwrap();
        session.fs().write(&h, 0, &[7u8; 4096]).await.unwrap();
    });

    let mut group = c.benchmark_group("session_fs");
    group.bench_function("read_4kb", |b| {
        b.to_async(&rt).iter(|| async {
            let h = session
                .fs()
                .open(Path::new("/data"), OpenFlags::default())
                .await
                .unwrap();
            let data = session.fs().read(&h, 0, 4096).await.unwrap();
            session.fs().close(&h);
            data
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    session_create_benchmarks,
    session_exec_benchmarks,
    session_fs_benchmarks
);
criterion_main!(benches);
