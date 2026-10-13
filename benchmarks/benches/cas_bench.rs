use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sandbox_cas::{BlobStore, MemStore, DiskStore, PutOptions};
use sandbox_types::hash::ContentHash;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

fn cas_mem_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = Arc::new(MemStore::new());
    
    let mut group = c.benchmark_group("cas_mem");
    
    // Benchmark: Put small blob (< 64 KiB)
    group.bench_function("put_small", |b| {
        let data = vec![0u8; 1024]; // 1 KiB
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Put medium blob (64 KiB)
    group.bench_function("put_64kb", |b| {
        let data = vec![0u8; 64 * 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Put large blob (1 MiB) - uses chunking
    group.bench_function("put_1mb", |b| {
        let data = vec![0u8; 1024 * 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Get small blob
    let small_hash = rt.block_on(async {
        let data = b"small test data";
        store.put(data, PutOptions::default()).await.unwrap()
    });
    
    group.bench_function("get_small", |b| {
        let hash = small_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.get(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Get large blob
    let large_hash = rt.block_on(async {
        let data = vec![0u8; 1024 * 1024];
        store.put(&data, PutOptions::default()).await.unwrap()
    });
    
    group.bench_function("get_1mb", |b| {
        let hash = large_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.get(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Has check
    group.bench_function("has", |b| {
        let hash = small_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.has(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Delete
    group.bench_function("delete", |b| {
        let data = b"to delete";
        let hash = rt.block_on(async {
            store.put(data, PutOptions::default()).await.unwrap()
        });
        b.to_async(&rt).iter(|| async {
            let _ = store.delete(&hash).await.unwrap();
        });
    });
    
    // Benchmark: List (empty)
    group.bench_function("list_empty", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = store.list(None).await.unwrap();
        });
    });
    
    // Benchmark: List with items
    let _ = rt.block_on(async {
        for i in 0..100 {
            let data = format!("item {}", i).into_bytes();
            store.put(&data, PutOptions::default()).await.unwrap();
        }
    });
    
    group.bench_function("list_100", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = store.list(None).await.unwrap();
        });
    });
    
    group.finish();
}

fn cas_disk_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let store = Arc::new(rt.block_on(async { DiskStore::new(temp_dir.path()).await.unwrap() }));
    
    let mut group = c.benchmark_group("cas_disk");
    
    // Benchmark: Put small blob
    group.bench_function("put_small", |b| {
        let data = vec![0u8; 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Put medium blob (64 KiB)
    group.bench_function("put_64kb", |b| {
        let data = vec![0u8; 64 * 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Put large blob (1 MiB)
    group.bench_function("put_1mb", |b| {
        let data = vec![0u8; 1024 * 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Get small blob
    let small_hash = rt.block_on(async {
        let data = b"small test data";
        store.put(data, PutOptions::default()).await.unwrap()
    });
    
    group.bench_function("get_small", |b| {
        let hash = small_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.get(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Get large blob
    let large_hash = rt.block_on(async {
        let data = vec![0u8; 1024 * 1024];
        store.put(&data, PutOptions::default()).await.unwrap()
    });
    
    group.bench_function("get_1mb", |b| {
        let hash = large_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.get(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Has check
    group.bench_function("has", |b| {
        let hash = small_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.has(&hash).await.unwrap();
        });
    });
    
    group.finish();
}

fn cas_tiered_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let local = rt.block_on(async { DiskStore::new(temp_dir.path()).await.unwrap() });
    let remote = MemStore::new();
    let store = Arc::new(sandbox_cas::TieredStore::new(local, remote));
    
    let mut group = c.benchmark_group("cas_tiered");
    
    // Benchmark: Put small blob (goes to local)
    group.bench_function("put_small", |b| {
        let data = vec![0u8; 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    // Benchmark: Get from local cache
    let local_hash = rt.block_on(async {
        let data = b"cached locally";
        store.put(data, PutOptions::default()).await.unwrap()
    });
    
    group.bench_function("get_local", |b| {
        let hash = local_hash;
        b.to_async(&rt).iter(|| async {
            let _ = store.get(&hash).await.unwrap();
        });
    });
    
    // Benchmark: Put and get large (tests tiering)
    group.bench_function("put_1mb", |b| {
        let data = vec![0u8; 1024 * 1024];
        b.to_async(&rt).iter(|| async {
            let _ = store.put(&data, PutOptions::default()).await.unwrap();
        });
    });
    
    group.finish();
}

criterion_group!(benches, cas_mem_benchmarks, cas_disk_benchmarks, cas_tiered_benchmarks);
criterion_main!(benches);