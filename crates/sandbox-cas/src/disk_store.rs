use crate::blob_store::{BlobStore, BlobStoreError, BlobStoreResult, PutOptions};
use async_trait::async_trait;
use bytes::Bytes;
use fastcdc::v2020::FastCDC;
use sandbox_types::hash::ContentHash;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs as tokio_fs;
use tokio::io::AsyncReadExt;
use tracing::{debug, trace};

const SHARD_BITS: u8 = 4;
const SHARD_COUNT: usize = 1 << SHARD_BITS;
const CHUNK_THRESHOLD: usize = 64 * 1024;
/// FastCDC cut points for chunked blobs (PLAN §4.2). The average matches the
/// threshold, so an edit changes only the chunks around it (ADR 0017).
const CDC_MIN: usize = 16 * 1024;
const CDC_AVG: usize = 64 * 1024;
const CDC_MAX: usize = 256 * 1024;
/// The manifest version this store writes. A manifest with no version is the
/// bare array of hashes that version 1 wrote; it is still read (ADR 0017).
const MANIFEST_VERSION: u8 = 2;
/// Distinguishes temporary files written by concurrent writers in one process.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct DiskStore {
    root: PathBuf,
    shards: Arc<[tokio::sync::RwLock<()>; SHARD_COUNT]>,
}

impl DiskStore {
    pub fn new(root: impl AsRef<Path>) -> BlobStoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(BlobStoreError::Io)?;

        for i in 0..SHARD_COUNT {
            let shard_dir = root.join(format!("{:02x}", i));
            fs::create_dir_all(&shard_dir).map_err(BlobStoreError::Io)?;
        }

        let shards = std::array::from_fn(|_| tokio::sync::RwLock::new(()));

        Ok(Self {
            root,
            shards: Arc::new(shards),
        })
    }

    pub fn open(root: impl AsRef<Path>) -> BlobStoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        if !root.exists() {
            return Err(BlobStoreError::InvalidArgument(format!(
                "Store root does not exist: {}",
                root.display()
            )));
        }

        let shards = std::array::from_fn(|_| tokio::sync::RwLock::new(()));

        Ok(Self {
            root,
            shards: Arc::new(shards),
        })
    }

    fn shard_index(&self, hash: &ContentHash) -> usize {
        let bytes = hash.as_bytes();
        let first_byte = bytes[0];
        (first_byte as usize) & (SHARD_COUNT - 1)
    }

    fn shard_path(&self, hash: &ContentHash) -> PathBuf {
        let idx = self.shard_index(hash);
        self.root.join(format!("{:02x}", idx)).join(hash.to_hex())
    }

    /// The directory of a version 1 blob's chunks. Version 1 keyed chunks by the
    /// blob that wrote them, so identical chunks were stored once per blob.
    fn chunk_dir(&self, hash: &ContentHash) -> PathBuf {
        let idx = self.shard_index(hash);
        self.root
            .join(format!("{:02x}", idx))
            .join("chunks")
            .join(&hash.to_hex()[..2])
    }

    /// The file of a chunk in the store-wide chunk area. A chunk is named by its
    /// own hash, so identical chunks from different blobs are one file (ADR 0017).
    fn chunk_file(&self, chunk: &ContentHash) -> PathBuf {
        self.chunk_dir(chunk).join(chunk.to_hex())
    }

    /// Writes `data` to `path` through a temporary file and a rename, so a reader
    /// never sees a partly written file, and a crash leaves either the old file
    /// or the new one. Chunks are shared between blobs, so a truncated chunk
    /// would corrupt every blob that reuses it.
    async fn write_file(&self, path: &Path, data: &[u8]) -> BlobStoreResult<()> {
        if let Some(parent) = path.parent() {
            tokio_fs::create_dir_all(parent)
                .await
                .map_err(BlobStoreError::Io)?;
        }
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let mut temp = path.as_os_str().to_owned();
        temp.push(format!(".tmp-{}-{seq}", std::process::id()));
        let temp = PathBuf::from(temp);
        tokio_fs::write(&temp, data)
            .await
            .map_err(BlobStoreError::Io)?;
        if let Err(e) = tokio_fs::rename(&temp, path).await {
            let _ = tokio_fs::remove_file(&temp).await;
            return Err(BlobStoreError::Io(e));
        }
        Ok(())
    }

    async fn read_file(&self, path: &Path) -> BlobStoreResult<Bytes> {
        let mut file = tokio_fs::File::open(path)
            .await
            .map_err(BlobStoreError::Io)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .await
            .map_err(BlobStoreError::Io)?;
        Ok(Bytes::from(buf))
    }

    /// Existence check without taking the shard lock. The caller must hold
    /// the lock for this hash's shard (read or write).
    async fn has_unlocked(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        let path = self.shard_path(hash);
        if path.exists() {
            return Ok(true);
        }
        let manifest_path = path.with_extension("manifest");
        Ok(manifest_path.exists())
    }

    /// Stores a large blob as content-defined chunks. Each chunk goes to the
    /// store-wide chunk area unless it is already there, and the blob's manifest
    /// names the chunks in order (ADR 0017).
    async fn put_chunked(
        &self,
        hash: &ContentHash,
        data: &[u8],
        compress: bool,
    ) -> BlobStoreResult<()> {
        let mut chunk_hashes = Vec::new();
        for cut in FastCDC::new(data, CDC_MIN, CDC_AVG, CDC_MAX) {
            let chunk = &data[cut.offset..cut.offset + cut.length];
            let chunk_hash = ContentHash::new(chunk);
            let chunk_path = self.chunk_file(&chunk_hash);
            if !chunk_path.exists() {
                let framed = encode_blob(chunk, compress)?;
                self.write_file(&chunk_path, &framed).await?;
            }
            chunk_hashes.push(chunk_hash);
        }

        let manifest = serde_json::to_vec(&Manifest {
            version: MANIFEST_VERSION,
            chunks: chunk_hashes,
        })
        .map_err(|e| {
            BlobStoreError::Storage(format!("Failed to serialize chunk manifest: {}", e))
        })?;

        let manifest_path = self.shard_path(hash).with_extension("manifest");
        self.write_file(&manifest_path, &manifest).await?;

        Ok(())
    }

    async fn get_chunked(&self, hash: &ContentHash) -> BlobStoreResult<Option<Bytes>> {
        let manifest_path = self.shard_path(hash).with_extension("manifest");
        if !manifest_path.exists() {
            return Ok(None);
        }

        let manifest = parse_manifest(&self.read_file(&manifest_path).await?)?;
        let mut result = Vec::new();
        for chunk_hash in &manifest.chunks {
            let chunk = if manifest.version == LEGACY_MANIFEST {
                self.read_file(&self.chunk_dir(hash).join(chunk_hash.to_hex()))
                    .await?
            } else {
                decode_blob(&self.read_file(&self.chunk_file(chunk_hash)).await?)?
            };
            result.extend_from_slice(&chunk);
        }

        let data = Bytes::from(result);
        verify(hash, &data)?;
        Ok(Some(data))
    }
}

/// The manifest of a chunked blob (ADR 0017). Version 2 is this struct. A version
/// 1 manifest is a bare array of hashes, read by `parse_manifest`.
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u8,
    chunks: Vec<ContentHash>,
}

/// The version a bare-array manifest is read as.
const LEGACY_MANIFEST: u8 = 1;

/// A parsed manifest: its version and the chunk hashes in order.
struct ParsedManifest {
    version: u8,
    chunks: Vec<ContentHash>,
}

fn parse_manifest(bytes: &[u8]) -> BlobStoreResult<ParsedManifest> {
    if let Ok(chunks) = serde_json::from_slice::<Vec<ContentHash>>(bytes) {
        return Ok(ParsedManifest {
            version: LEGACY_MANIFEST,
            chunks,
        });
    }
    let manifest: Manifest = serde_json::from_slice(bytes)
        .map_err(|e| BlobStoreError::Storage(format!("Failed to parse chunk manifest: {}", e)))?;
    if manifest.version != MANIFEST_VERSION {
        return Err(BlobStoreError::Storage(format!(
            "unknown chunk manifest version {}",
            manifest.version
        )));
    }
    Ok(ParsedManifest {
        version: manifest.version,
        chunks: manifest.chunks,
    })
}

/// Fails when `data` does not hash to `hash`. A read never returns bytes that
/// are not the blob that was asked for, so corruption is an error (PLAN §7).
fn verify(hash: &ContentHash, data: &[u8]) -> BlobStoreResult<()> {
    if ContentHash::new(data) == *hash {
        Ok(())
    } else {
        Err(BlobStoreError::Storage(format!(
            "blob {hash} does not match its hash; the store is corrupt"
        )))
    }
}

/// Leading byte of every single-file blob: says whether the payload is zstd.
const FRAME_RAW: u8 = 0x00;
const FRAME_ZSTD: u8 = 0x01;

/// Frames a blob for disk. The flag byte lets `get` decode without guessing
/// from the content, which could itself look like a zstd header.
fn encode_blob(data: &[u8], compress: bool) -> BlobStoreResult<Vec<u8>> {
    if compress {
        let payload = zstd::encode_all(data, 3)
            .map_err(|e| BlobStoreError::Compression(format!("zstd compression failed: {}", e)))?;
        let mut framed = Vec::with_capacity(payload.len() + 1);
        framed.push(FRAME_ZSTD);
        framed.extend_from_slice(&payload);
        Ok(framed)
    } else {
        let mut framed = Vec::with_capacity(data.len() + 1);
        framed.push(FRAME_RAW);
        framed.extend_from_slice(data);
        Ok(framed)
    }
}

fn decode_blob(framed: &[u8]) -> BlobStoreResult<Bytes> {
    match framed.split_first() {
        Some((&FRAME_RAW, payload)) => Ok(Bytes::copy_from_slice(payload)),
        Some((&FRAME_ZSTD, payload)) => zstd::decode_all(payload)
            .map(Bytes::from)
            .map_err(|e| BlobStoreError::Compression(format!("zstd decompression failed: {}", e))),
        _ => Err(BlobStoreError::Storage(
            "blob has an unknown frame header".to_string(),
        )),
    }
}

#[async_trait]
impl BlobStore for DiskStore {
    async fn get(&self, hash: &ContentHash) -> BlobStoreResult<Option<Bytes>> {
        let _guard = self.shards[self.shard_index(hash)].read().await;
        trace!("DiskStore get: {}", hash);

        let path = self.shard_path(hash);
        if path.exists() {
            let framed = self.read_file(&path).await?;
            let data = decode_blob(&framed)?;
            verify(hash, &data)?;
            return Ok(Some(data));
        }

        self.get_chunked(hash).await
    }

    async fn put(&self, data: &[u8], options: PutOptions) -> BlobStoreResult<ContentHash> {
        let hash = ContentHash::new(data);
        let _guard = self.shards[self.shard_index(&hash)].write().await;

        // Use the unlocked check: `has` takes a read lock on this same shard,
        // and tokio's RwLock is not reentrant, so calling it here deadlocks.
        if self.has_unlocked(&hash).await? {
            return Ok(hash);
        }

        let should_chunk = options.chunk_if_large && data.len() > CHUNK_THRESHOLD;

        if should_chunk {
            self.put_chunked(&hash, data, options.compress).await?;
        } else {
            let final_data = encode_blob(data, options.compress)?;
            let path = self.shard_path(&hash);
            self.write_file(&path, &final_data).await?;
        }

        debug!("DiskStore put: {} ({} bytes)", hash, data.len());
        Ok(hash)
    }

    async fn has(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        let _guard = self.shards[self.shard_index(hash)].read().await;
        self.has_unlocked(hash).await
    }

    async fn delete(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        let _guard = self.shards[self.shard_index(hash)].write().await;
        let path = self.shard_path(hash);
        let manifest_path = path.with_extension("manifest");

        let mut deleted = false;
        if path.exists() {
            tokio_fs::remove_file(&path)
                .await
                .map_err(BlobStoreError::Io)?;
            deleted = true;
        }
        if manifest_path.exists() {
            tokio_fs::remove_file(&manifest_path)
                .await
                .map_err(BlobStoreError::Io)?;
            deleted = true;
        }

        Ok(deleted)
    }

    async fn list_parts(&self) -> BlobStoreResult<Vec<String>> {
        let mut keys = Vec::new();
        for i in 0..SHARD_COUNT {
            let chunks = self.root.join(format!("{:02x}", i)).join("chunks");
            if !chunks.exists() {
                continue;
            }
            let mut prefixes = tokio_fs::read_dir(&chunks)
                .await
                .map_err(BlobStoreError::Io)?;
            while let Some(prefix) = prefixes.next_entry().await.map_err(BlobStoreError::Io)? {
                let mut files = tokio_fs::read_dir(prefix.path())
                    .await
                    .map_err(BlobStoreError::Io)?;
                while let Some(file) = files.next_entry().await.map_err(BlobStoreError::Io)? {
                    // A chunk is named by its hash. Anything else is a temporary
                    // file from a write that did not finish, and is not a sweep
                    // candidate (`delete_parts` would reject its name).
                    let name = file.file_name().to_string_lossy().to_string();
                    if ContentHash::from_hex(&name).is_err() {
                        continue;
                    }
                    keys.push(format!(
                        "{:02x}/chunks/{}/{}",
                        i,
                        prefix.file_name().to_string_lossy(),
                        name
                    ));
                }
            }
        }
        Ok(keys)
    }

    async fn parts(&self, hash: &ContentHash) -> BlobStoreResult<Vec<ContentHash>> {
        let manifest_path = self.shard_path(hash).with_extension("manifest");
        if !manifest_path.exists() {
            return Ok(Vec::new());
        }
        let manifest = self.read_file(&manifest_path).await?;
        Ok(parse_manifest(&manifest)?.chunks)
    }

    async fn delete_parts(
        &self,
        candidates: &[String],
        keep: &HashSet<ContentHash>,
    ) -> BlobStoreResult<usize> {
        let mut removed = 0;
        for key in candidates {
            // A key is `shard/chunks/xx/hash`, as `list_parts` builds it.
            let mut segments = key.rsplit('/');
            let Some(hash) = segments
                .next()
                .and_then(|hex| ContentHash::from_hex(hex).ok())
            else {
                return Err(BlobStoreError::InvalidArgument(format!(
                    "not a chunk key: {key}"
                )));
            };
            if keep.contains(&hash) {
                continue;
            }
            let shard = key
                .split('/')
                .next()
                .and_then(|s| usize::from_str_radix(s, 16).ok());
            let Some(shard) = shard.filter(|s| *s < SHARD_COUNT) else {
                return Err(BlobStoreError::InvalidArgument(format!(
                    "not a chunk key: {key}"
                )));
            };
            let _guard = self.shards[shard].write().await;
            let path = self.root.join(key);
            if path.exists() {
                tokio_fs::remove_file(&path)
                    .await
                    .map_err(BlobStoreError::Io)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    async fn list(&self, prefix: Option<&str>) -> BlobStoreResult<Vec<ContentHash>> {
        let mut results = Vec::new();
        let prefix_str = prefix.unwrap_or("");

        for i in 0..SHARD_COUNT {
            let shard_dir = self.root.join(format!("{:02x}", i));
            if !shard_dir.exists() {
                continue;
            }

            let mut entries = tokio_fs::read_dir(&shard_dir)
                .await
                .map_err(BlobStoreError::Io)?;
            while let Some(entry) = entries.next_entry().await.map_err(BlobStoreError::Io)? {
                // A chunked blob has a manifest and no file of its own, so the
                // manifest's name is the blob's hash.
                let name = entry.file_name().to_string_lossy().to_string();
                let hex = name.strip_suffix(".manifest").unwrap_or(&name);
                if !hex.starts_with(prefix_str) {
                    continue;
                }
                if let Ok(hash) = ContentHash::from_hex(hex) {
                    results.push(hash);
                }
            }
        }

        Ok(results)
    }

    fn name(&self) -> &'static str {
        "disk"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_store::PutOptions;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_disk_store_basic() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let data = b"hello world";

        let hash = store.put(data, PutOptions::default()).await.unwrap();
        assert!(store.has(&hash).await.unwrap());

        let retrieved = store.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved.as_ref(), data);

        let deleted = store.delete(&hash).await.unwrap();
        assert!(deleted);
        assert!(!store.has(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_disk_store_put_does_not_deadlock() {
        // Regression: put() used to re-acquire its own shard lock via has().
        // The timeout turns a reintroduced deadlock into a failure.
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();

        let first = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.put(b"dedupe me", PutOptions::default()),
        )
        .await
        .expect("put deadlocked on first write")
        .unwrap();
        let second = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.put(b"dedupe me", PutOptions::default()),
        )
        .await
        .expect("put deadlocked on duplicate write")
        .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn test_disk_store_chunked() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();

        let data = vec![0x42; 100 * 1024];
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };

        let hash = store.put(&data, options).await.unwrap();
        assert!(store.has(&hash).await.unwrap());

        let retrieved = store.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved.as_ref(), data.as_slice());
    }

    #[tokio::test]
    async fn test_disk_store_compression() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();

        let data = "highly compressible data ".repeat(1000).into_bytes();
        let options = PutOptions {
            compress: true,
            chunk_if_large: false,
        };

        let hash = store.put(&data, options).await.unwrap();
        let retrieved = store.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved.as_ref(), data.as_slice());
    }

    #[tokio::test]
    async fn test_disk_store_list() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();

        let hash1 = store.put(b"data1", PutOptions::default()).await.unwrap();
        let hash2 = store.put(b"data2", PutOptions::default()).await.unwrap();

        let list = store.list(None).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&hash1));
        assert!(list.contains(&hash2));
    }

    /// Deterministic bytes that do not repeat within a chunk, so each chunk has
    /// its own content hash (xorshift64).
    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    /// Bytes of chunk files in the store, as they are on disk.
    async fn chunk_bytes(store: &DiskStore) -> u64 {
        let mut total = 0;
        for key in store.list_parts().await.unwrap() {
            total += std::fs::metadata(store.root.join(key)).unwrap().len();
        }
        total
    }

    #[tokio::test]
    async fn a_chunk_is_kept_while_a_blob_names_it_and_removed_after() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        // Two blobs with the same first 512 KiB and different tails.
        let common = pseudo_random(512 * 1024, 1);
        let mut first = common.clone();
        first.extend(b"the tail of the first blob");
        let mut second = common.clone();
        second.extend(b"a different tail for the second");
        let h1 = store.put(&first, options).await.unwrap();
        let h2 = store.put(&second, options).await.unwrap();

        let parts1 = store.parts(&h1).await.unwrap();
        let parts2 = store.parts(&h2).await.unwrap();
        let shared: Vec<ContentHash> = parts1
            .iter()
            .copied()
            .filter(|p| parts2.contains(p))
            .collect();
        let only_first: Vec<ContentHash> = parts1
            .iter()
            .copied()
            .filter(|p| !parts2.contains(p))
            .collect();
        assert!(
            !shared.is_empty(),
            "blobs with a common prefix share chunks"
        );
        assert!(!only_first.is_empty(), "the first tail has its own chunk");

        // Deleting the first blob leaves its chunks on disk until a sweep.
        assert!(store.delete(&h1).await.unwrap());
        let candidates = store.list_parts().await.unwrap();
        let blobs = store.list(None).await.unwrap();
        assert_eq!(blobs, vec![h2], "a chunked blob is listed by its manifest");
        let mut keep = HashSet::new();
        for hash in blobs {
            keep.extend(store.parts(&hash).await.unwrap());
        }
        let removed = store.delete_parts(&candidates, &keep).await.unwrap();
        assert_eq!(removed, only_first.len(), "only the first tail's chunks go");

        assert_eq!(
            store.get(&h2).await.unwrap().unwrap().as_ref(),
            second.as_slice(),
            "the shared chunks must survive for the second blob"
        );
        let left = store.list_parts().await.unwrap();
        for chunk in &only_first {
            assert!(!left.iter().any(|key| key.ends_with(&chunk.to_hex())));
        }
        for chunk in &shared {
            assert!(left.iter().any(|key| key.ends_with(&chunk.to_hex())));
        }
    }

    #[tokio::test]
    async fn identical_chunks_from_different_blobs_are_stored_once() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        let common = pseudo_random(512 * 1024, 2);
        let mut first = common.clone();
        first.push(1);
        let mut second = common.clone();
        second.push(2);
        let h1 = store.put(&first, options).await.unwrap();
        let h2 = store.put(&second, options).await.unwrap();

        let mut unique: HashSet<ContentHash> =
            store.parts(&h1).await.unwrap().into_iter().collect();
        unique.extend(store.parts(&h2).await.unwrap());
        assert_eq!(
            store.list_parts().await.unwrap().len(),
            unique.len(),
            "one chunk file per distinct chunk"
        );
    }

    #[tokio::test]
    async fn an_edit_stores_only_the_chunks_around_it() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        let original = pseudo_random(1024 * 1024, 3);
        store.put(&original, options).await.unwrap();
        let before = chunk_bytes(&store).await;

        let mut edited = original.clone();
        edited.splice(500_000..500_000, b"a short insertion".iter().copied());
        store.put(&edited, options).await.unwrap();
        let added = chunk_bytes(&store).await - before;

        // Fixed-size chunks would store the whole edited blob again. Content-
        // defined chunks store the one or two chunks that contain the edit.
        assert!(
            added < original.len() as u64 / 4,
            "an insertion added {added} bytes of chunks to a {} byte blob",
            original.len()
        );
    }

    #[tokio::test]
    async fn compressed_chunks_are_smaller_on_disk() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let data = "highly compressible text ".repeat(40_000).into_bytes();
        let options = PutOptions {
            compress: true,
            chunk_if_large: true,
        };
        let hash = store.put(&data, options).await.unwrap();
        assert!(!store.parts(&hash).await.unwrap().is_empty());
        assert!(
            chunk_bytes(&store).await < data.len() as u64 / 4,
            "compressible chunks must be stored compressed"
        );
        assert_eq!(
            store.get(&hash).await.unwrap().unwrap().as_ref(),
            data.as_slice()
        );
    }

    #[tokio::test]
    async fn a_corrupt_chunk_is_an_error_not_wrong_bytes() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        let data = pseudo_random(256 * 1024, 4);
        let hash = store.put(&data, options).await.unwrap();
        let chunk = store.parts(&hash).await.unwrap()[0];
        // A well-formed frame with the wrong content: decoding succeeds, so only
        // the hash check can catch it.
        let wrong = encode_blob(b"not the chunk that was stored", false).unwrap();
        std::fs::write(store.chunk_file(&chunk), wrong).unwrap();

        let err = store.get(&hash).await.unwrap_err();
        assert!(err.to_string().contains("does not match its hash"), "{err}");
    }

    #[tokio::test]
    async fn a_version_one_manifest_is_still_read() {
        // Version 1 wrote fixed 64 KiB chunks, raw, under the blob's own chunk
        // directory, and a bare array of hashes as the manifest.
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let data = pseudo_random(200 * 1024, 5);
        let hash = ContentHash::new(&data);
        let mut chunk_hashes = Vec::new();
        for piece in data.chunks(CHUNK_THRESHOLD) {
            let chunk = ContentHash::new(piece);
            store
                .write_file(&store.chunk_dir(&hash).join(chunk.to_hex()), piece)
                .await
                .unwrap();
            chunk_hashes.push(chunk);
        }
        let manifest = serde_json::to_vec(&chunk_hashes).unwrap();
        store
            .write_file(
                &store.shard_path(&hash).with_extension("manifest"),
                &manifest,
            )
            .await
            .unwrap();

        assert_eq!(
            store.get(&hash).await.unwrap().unwrap().as_ref(),
            data.as_slice()
        );
        assert_eq!(store.parts(&hash).await.unwrap(), chunk_hashes);
    }

    #[tokio::test]
    async fn a_temporary_file_is_not_a_sweep_candidate() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        store
            .put(&pseudo_random(128 * 1024, 6), options)
            .await
            .unwrap();
        let chunk = store.list_parts().await.unwrap()[0].clone();
        // A write that died before its rename leaves a file next to the chunk.
        let leftover = store.root.join(format!("{chunk}.tmp-999-0"));
        std::fs::write(&leftover, b"partial").unwrap();

        let keys = store.list_parts().await.unwrap();
        assert!(keys.iter().all(|key| !key.contains(".tmp-")));
        assert!(store.delete_parts(&keys, &HashSet::new()).await.is_ok());
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;
        use proptest::test_runner::TestCaseError;

        fn runtime() -> tokio::runtime::Runtime {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(24))]

            /// Any blob reads back byte for byte under its own hash, whether it
            /// is stored whole or chunked, and with or without compression.
            #[test]
            fn every_blob_round_trips(
                data in proptest::collection::vec(any::<u8>(), 0..300_000),
                compress in any::<bool>(),
                chunk_if_large in any::<bool>(),
            ) {
                runtime().block_on(async {
                    let dir = tempdir().unwrap();
                    let store = DiskStore::new(dir.path()).unwrap();
                    let options = PutOptions { compress, chunk_if_large };
                    let hash = store.put(&data, options).await.unwrap();
                    prop_assert_eq!(hash, ContentHash::new(&data));
                    let read = store.get(&hash).await.unwrap().unwrap();
                    prop_assert_eq!(read.as_ref(), data.as_slice());
                    // A chunked blob's chunk files, in order, make up the blob.
                    let parts = store.parts(&hash).await.unwrap();
                    if !parts.is_empty() {
                        let mut joined = Vec::new();
                        for chunk in parts {
                            let framed = std::fs::read(store.chunk_file(&chunk)).unwrap();
                            joined.extend_from_slice(&decode_blob(&framed).unwrap());
                        }
                        prop_assert_eq!(joined, data.clone());
                    }
                    Ok::<(), TestCaseError>(())
                })?;
            }

            /// An insertion into a megabyte blob stores only the chunks that
            /// contain it, so the new bytes on disk are a small part of the blob.
            #[test]
            fn an_insertion_stores_a_small_part_of_the_blob(
                len in 1_048_576usize..2_097_152,
                at in 0usize..1_048_576,
                insert in proptest::collection::vec(any::<u8>(), 1..4096),
                seed in any::<u64>(),
            ) {
                let original = pseudo_random(len, seed);
                let mut edited = original.clone();
                edited.splice(at..at, insert.iter().copied());
                runtime().block_on(async {
                    let dir = tempdir().unwrap();
                    let store = DiskStore::new(dir.path()).unwrap();
                    let options = PutOptions { compress: false, chunk_if_large: true };
                    store.put(&original, options).await.unwrap();
                    let before = chunk_bytes(&store).await;
                    store.put(&edited, options).await.unwrap();
                    let added = chunk_bytes(&store).await - before;
                    prop_assert!(
                        added < len as u64 / 4,
                        "an insertion of {} bytes added {} bytes of chunks to {} bytes",
                        insert.len(), added, len
                    );
                    Ok::<(), TestCaseError>(())
                })?;
            }

            /// A sweep keeps every chunk that a live blob names, and removes
            /// every other chunk. Nothing live is lost.
            #[test]
            fn a_sweep_keeps_what_live_blobs_name(
                blobs in proptest::collection::vec(
                    proptest::collection::vec(any::<u8>(), 60_000..200_000), 2..5),
                live_mask in proptest::collection::vec(any::<bool>(), 5),
            ) {
                runtime().block_on(async {
                    let dir = tempdir().unwrap();
                    let store = DiskStore::new(dir.path()).unwrap();
                    let options = PutOptions { compress: false, chunk_if_large: true };
                    let mut hashes = Vec::new();
                    for blob in &blobs {
                        hashes.push(store.put(blob, options).await.unwrap());
                    }
                    let mut keep = HashSet::new();
                    let mut live = Vec::new();
                    for (i, hash) in hashes.iter().enumerate() {
                        if live_mask[i] {
                            keep.extend(store.parts(hash).await.unwrap());
                            live.push(*hash);
                        }
                    }
                    for hash in &hashes {
                        if !live.contains(hash) {
                            store.delete(hash).await.unwrap();
                        }
                    }
                    let candidates = store.list_parts().await.unwrap();
                    store.delete_parts(&candidates, &keep).await.unwrap();
                    for (i, hash) in hashes.iter().enumerate() {
                        if live_mask[i] {
                            let read = store.get(hash).await.unwrap().unwrap();
                            prop_assert_eq!(read.as_ref(), blobs[i].as_slice());
                        }
                    }
                    Ok::<(), TestCaseError>(())
                })?;
            }
        }
    }

    #[tokio::test]
    async fn an_empty_keep_set_removes_every_chunk_file() {
        let dir = tempdir().unwrap();
        let store = DiskStore::new(dir.path()).unwrap();
        let options = PutOptions {
            compress: false,
            chunk_if_large: true,
        };
        store.put(&vec![7u8; 100 * 1024], options).await.unwrap();
        let candidates = store.list_parts().await.unwrap();
        assert!(!candidates.is_empty());
        let removed = store
            .delete_parts(&candidates, &HashSet::new())
            .await
            .unwrap();
        assert_eq!(removed, candidates.len());
        assert!(store.list_parts().await.unwrap().is_empty());
    }
}
