//! VFS micro-benchmarks.
//!
//! Commit benches build a fresh VFS inside each iteration because `commit`
//! takes `&mut self` and `Vfs` is not `Clone`. Their numbers include setup.

use criterion::{criterion_group, criterion_main, Criterion};
use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::path::Path;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn new_vfs() -> Vfs {
    Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()))
}

fn create_flags() -> OpenFlags {
    OpenFlags {
        read: true,
        write: true,
        create: true,
        ..OpenFlags::default()
    }
}

fn read_flags() -> OpenFlags {
    OpenFlags::default()
}

fn vfs_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("vfs");

    group.bench_function("create", |b| b.iter(new_vfs));

    group.bench_function("stat_root", |b| {
        let vfs = new_vfs();
        b.to_async(&rt).iter(|| async {
            let _ = vfs.stat(Path::new("/")).await;
        });
    });

    group.bench_function("mkdir", |b| {
        b.to_async(&rt).iter(|| async {
            let vfs = new_vfs();
            vfs.mkdir(Path::new("/testdir"), 0o755).await.unwrap();
        });
    });

    group.bench_function("create_and_write", |b| {
        b.to_async(&rt).iter(|| async {
            let vfs = new_vfs();
            let handle = vfs
                .open(Path::new("/test.txt"), create_flags())
                .await
                .unwrap();
            vfs.write(&handle, 0, b"hello world").await.unwrap();
        });
    });

    // Target: < 5 µs (see docs/PERFORMANCE.md).
    group.bench_function("read_4kb_overlay", |b| {
        let vfs = new_vfs();
        let handle = rt.block_on(async {
            let h = vfs
                .open(Path::new("/data.bin"), create_flags())
                .await
                .unwrap();
            vfs.write(&h, 0, &vec![0u8; 4096]).await.unwrap();
            vfs.open(Path::new("/data.bin"), read_flags())
                .await
                .unwrap()
        });
        b.to_async(&rt).iter(|| async {
            let _ = vfs.read(&handle, 0, 4096).await;
        });
    });

    group.bench_function("readdir_100", |b| {
        let vfs = new_vfs();
        rt.block_on(async {
            vfs.mkdir(Path::new("/dir"), 0o755).await.unwrap();
            for i in 0..100 {
                let p = format!("/dir/file_{i}");
                let h = vfs.open(Path::new(&p), create_flags()).await.unwrap();
                vfs.write(&h, 0, b"x").await.unwrap();
            }
        });
        b.to_async(&rt).iter(|| async {
            let _ = vfs.readdir(Path::new("/dir")).await;
        });
    });

    group.bench_function("commit_small", |b| {
        b.to_async(&rt).iter(|| async {
            let mut vfs = new_vfs();
            let h = vfs
                .open(Path::new("/test.txt"), create_flags())
                .await
                .unwrap();
            vfs.write(&h, 0, b"small content").await.unwrap();
            let _ = vfs.commit().await;
        });
    });

    group.bench_function("commit_10_files", |b| {
        b.to_async(&rt).iter(|| async {
            let mut vfs = new_vfs();
            for i in 0..10 {
                let p = format!("/file_{i}.txt");
                let h = vfs.open(Path::new(&p), create_flags()).await.unwrap();
                vfs.write(&h, 0, format!("content {i}").as_bytes())
                    .await
                    .unwrap();
            }
            let _ = vfs.commit().await;
        });
    });

    group.finish();
}

fn vfs_overlay_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("vfs_overlay");

    group.bench_function("write_new_file", |b| {
        let vfs = new_vfs();
        rt.block_on(async { vfs.mkdir(Path::new("/dir"), 0o755).await.unwrap() });
        let mut n = 0u64;
        b.to_async(&rt).iter(|| {
            n += 1;
            let path = format!("/dir/f{n}");
            let vfs = &vfs;
            async move {
                let h = vfs.open(Path::new(&path), create_flags()).await.unwrap();
                let _ = vfs.write(&h, 0, b"new content").await;
            }
        });
    });

    group.bench_function("rename_file", |b| {
        b.to_async(&rt).iter(|| async {
            let vfs = new_vfs();
            let h = vfs
                .open(Path::new("/source.txt"), create_flags())
                .await
                .unwrap();
            vfs.write(&h, 0, b"content").await.unwrap();
            let _ = vfs
                .rename(Path::new("/source.txt"), Path::new("/dest.txt"))
                .await;
        });
    });

    group.bench_function("symlink", |b| {
        b.to_async(&rt).iter(|| async {
            let vfs = new_vfs();
            let _ = vfs
                .symlink(Path::new("/target.txt"), Path::new("/link.txt"))
                .await;
        });
    });

    group.finish();
}

criterion_group!(benches, vfs_benchmarks, vfs_overlay_benchmarks);
criterion_main!(benches);
