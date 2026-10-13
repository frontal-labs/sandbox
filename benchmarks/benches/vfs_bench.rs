use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sandbox_vfs::{Vfs, VfsConfig};
use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn vfs_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = Arc::new(MemStore::new());
    let root_hash = ContentHash::nil();
    
    let mut group = c.benchmark_group("vfs");
    
    // Benchmark: VFS creation
    group.bench_function("create", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _vfs = Vfs::new(root_hash, store.clone());
            });
        });
    });
    
    // Benchmark: Stat operation (empty VFS)
    group.bench_function("stat_empty", |b| {
        let vfs = rt.block_on(async { Vfs::new(root_hash, store.clone()) });
        b.to_async(&rt).iter(|| async {
            let _ = vfs.stat(std::path::Path::new("/")).await;
        });
    });
    
    // Benchmark: Mkdir
    group.bench_function("mkdir", |b| {
        let vfs = rt.block_on(async { Vfs::new(root_hash, store.clone()) });
        b.to_async(&rt).iter(|| async {
            let _ = vfs.mkdir(std::path::Path::new("/testdir"), 0o755).await;
        });
    });
    
    // Benchmark: Create file and write
    group.bench_function("create_and_write", |b| {
        let vfs = rt.block_on(async { Vfs::new(root_hash, store.clone()) });
        b.to_async(&rt).iter(|| async {
            let handle = vfs.open(std::path::Path::new("/test.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            let _ = vfs.write(&handle, 0, b"hello world").await;
        });
    });
    
    // Benchmark: Read 4KB (cached) - Target: < 5 µs
    group.bench_function("read_4kb_cached", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            let handle = vfs.open(std::path::Path::new("/data.bin"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            let data = vec![0u8; 4096];
            vfs.write(&handle, 0, &data).await.unwrap();
            vfs
        });
        
        let handle = rt.block_on(async {
            vfs.open(std::path::Path::new("/data.bin"), sandbox_vfs::ops::OpenFlags::empty()).await.unwrap()
        });
        
        b.to_async(&rt).iter(|| async {
            let _ = vfs.read(&handle, 0, 4096).await;
        });
    });
    
    // Benchmark: Readdir
    group.bench_function("readdir", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            for i in 0..100 {
                vfs.mkdir(std::path::Path::new(&format!("/dir/file_{}", i)), 0o755).await.unwrap();
            }
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let _ = vfs.readdir(std::path::Path::new("/dir")).await;
        });
    });
    
    // Benchmark: Commit (small overlay)
    group.bench_function("commit_small", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            let handle = vfs.open(std::path::Path::new("/test.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            vfs.write(&handle, 0, b"small content").await.unwrap();
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let mut vfs_clone = vfs.clone();
            let _ = vfs_clone.commit().await;
        });
    });
    
    // Benchmark: Commit with multiple files
    group.bench_function("commit_multiple", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            for i in 0..10 {
                let handle = vfs.open(std::path::Path::new(&format!("/file_{}.txt", i)), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
                vfs.write(&handle, 0, format!("content {}", i).as_bytes()).await.unwrap();
            }
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let mut vfs_clone = vfs.clone();
            let _ = vfs_clone.commit().await;
        });
    });
    
    group.finish();
}

fn vfs_overlay_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = Arc::new(MemStore::new());
    let root_hash = ContentHash::nil();
    
    let mut group = c.benchmark_group("vfs_overlay");
    
    // Benchmark: Overlay read (checks overlay first, then base)
    group.bench_function("overlay_read_hit", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            let handle = vfs.open(std::path::Path::new("/overlay.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            vfs.write(&handle, 0, b"overlay content").await.unwrap();
            vfs
        });
        
        let handle = rt.block_on(async {
            vfs.open(std::path::Path::new("/overlay.txt"), sandbox_vfs::ops::OpenFlags::empty()).await.unwrap()
        });
        
        b.to_async(&rt).iter(|| async {
            let _ = vfs.read(&handle, 0, 16).await;
        });
    });
    
    // Benchmark: Overlay write
    group.bench_function("overlay_write", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            vfs.mkdir(std::path::Path::new("/dir"), 0o755).await.unwrap();
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let handle = vfs.open(std::path::Path::new("/dir/new_file.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            let _ = vfs.write(&handle, 0, b"new content").await;
        });
    });
    
    // Benchmark: Rename
    group.bench_function("rename", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            let handle = vfs.open(std::path::Path::new("/source.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            vfs.write(&handle, 0, b"content").await.unwrap();
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let _ = vfs.rename(std::path::Path::new("/source.txt"), std::path::Path::new("/dest.txt")).await;
        });
    });
    
    // Benchmark: Symlink
    group.bench_function("symlink", |b| {
        let vfs = rt.block_on(async { 
            let vfs = Vfs::new(root_hash, store.clone());
            let handle = vfs.open(std::path::Path::new("/target.txt"), sandbox_vfs::ops::OpenFlags::CREATE).await.unwrap();
            vfs.write(&handle, 0, b"target").await.unwrap();
            vfs
        });
        
        b.to_async(&rt).iter(|| async {
            let _ = vfs.symlink(std::path::Path::new("/target.txt"), std::path::Path::new("/link.txt")).await;
        });
    });
    
    group.finish();
}

criterion_group!(benches, vfs_benchmarks, vfs_overlay_benchmarks);
criterion_main!(benches);