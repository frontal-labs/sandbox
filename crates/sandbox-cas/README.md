# sandbox-cas

Content-addressed blob store for the sandbox system. Blobs are keyed by the BLAKE3 hash of their uncompressed content.

**Status: partial.** `MemStore`, `DiskStore`, and `TieredStore` are implemented and have unit tests. There is no standalone object-store backend, and there is no refcounting or GC yet.

## Public API

Re-exported from `src/lib.rs`:

- `trait BlobStore` (`src/blob_store.rs`): `get(&ContentHash)`, `put(&[u8], PutOptions) -> ContentHash`, `has`, `delete`, `list(prefix)`, `name`.
- `PutOptions { compress, chunk_if_large }` (both default to `true`).
- `BlobStoreError`, `BlobStoreResult<T>`.
- `MemStore`: in-memory map (`new`, `with_capacity`, `len`, `is_empty`, `clear`).
- `DiskStore`: `new(root)` and `open(root)`, both synchronous. Uses 256 shard directories (`00`-`ff`) with per-shard locks.
- `TieredStore`: `new(local, remote: Arc<dyn object_store::ObjectStore>, CachePolicy)`, plus `stats()`, `local_store()`, `remote_store()`.

## Behaviour

- **Hashing:** `ContentHash::new(data)` is computed over the uncompressed bytes before any compression.
- **Compression:** when `compress` is set, `DiskStore` writes zstd level 3.
- **Chunking:** when `chunk_if_large` is set and the data is over 64 KiB, `DiskStore` splits it into fixed 64 KiB chunks and writes a JSON manifest. It does not use FastCDC, although `fastcdc` is listed as a dependency and is not yet used.
- **Not yet implemented:** mmap reads, refcounting, mark-and-sweep GC, and an `ObjectStore`-backed `BlobStore`. `TieredStore` uses `object_store` directly.

## Invariants

- A `put` of identical bytes returns the same hash. `DiskStore` skips the write if the hash already exists.
- The hash is always over uncompressed content, so compression settings do not change identity.

## Security assumptions

- The store trusts its caller to supply correct hashes. `get` does not currently re-verify the hash of returned bytes. Treat this as a gap to close before the CAS serves untrusted input.
- `DiskStore` writes under a host directory that the caller chooses. It does not sandbox that directory.

## Example

```rust
use sandbox_cas::{BlobStore, DiskStore, PutOptions};

async fn roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let store = DiskStore::new("/data/cas")?;
    let hash = store.put(b"hello world", PutOptions::default()).await?;
    assert!(store.has(&hash).await?);
    Ok(())
}
```

Not compiled in this pass. Checked against the signatures in `src/blob_store.rs` and `src/disk_store.rs`.

## Spec

See [PLAN.md](../../PLAN.md) section 4.2 for the target design. Still planned: `ObjectStore` backend, FastCDC chunking, refcounting, and GC.
