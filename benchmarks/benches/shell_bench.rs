//! Shell micro-benchmarks: parsing and running scripts against an in-memory VFS.

use criterion::{criterion_group, criterion_main, Criterion};
use sandbox_cas::MemStore;
use sandbox_shell::{parse, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

fn new_shell() -> Shell {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    Shell::new(fs, ShellConfig::default())
}

fn shell_parser_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("shell_parser");
    group.bench_function("simple", |b| b.iter(|| parse("echo hello").unwrap()));
    group.bench_function("pipeline", |b| {
        b.iter(|| parse("echo hello | grep h | cat").unwrap())
    });
    group.bench_function("operators_and_redirects", |b| {
        b.iter(|| parse("mkdir -p /a && echo x > /a/f || echo no; cat < /a/f >> /a/g").unwrap())
    });
    group.finish();
}

fn shell_exec_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("shell_exec");

    // The shell is behind a Mutex because the async iter closure is FnMut.
    group.bench_function("echo", |b| {
        let sh = Mutex::new(new_shell());
        b.to_async(&rt)
            .iter(|| async { sh.lock().await.run("echo hello").await.unwrap() });
    });

    group.bench_function("pipe_and_redirect", |b| {
        let sh = Mutex::new(new_shell());
        b.to_async(&rt).iter(|| async {
            sh.lock()
                .await
                .run("echo hi | cat > /a && cat /a")
                .await
                .unwrap()
        });
    });

    group.finish();
}

criterion_group!(benches, shell_parser_benchmarks, shell_exec_benchmarks);
criterion_main!(benches);
