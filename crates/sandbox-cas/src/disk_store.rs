use crate::blob_store::{BlobStore, BlobStoreError, BlobStoreResult, PutOptions};
use async_trait::async_trait;
use bytes::Bytes;
use sandbox_types::hash::ContentHash;
use serde_json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs as tokio_fs;
use tokio::io::AsyncReadExt;
use tracing::{debug, trace};

const SHARD_BITS: u8 = 4;
const SHARD_COUNT: usize = 1 << SHARD_BITS;
const CHUNK_THRESHOLD: usize = 64 * 1024;

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

    fn chunk_dir(&self, hash: &ContentHash) -> PathBuf {
        let idx = self.shard_index(hash);
        self.root
            .join(format!("{:02x}", idx))
            .join("chunks")
            .join(&hash.to_hex()[..2])
    }

    async fn write_file(&self, path: &Path, data: &[u8]) -> BlobStoreResult<()> {
        if let Some(parent) = path.parent() {
            tokio_fs::create_dir_all(parent)
                .await
                .map_err(BlobStoreError::Io)?;
        }
        tokio_fs::write(path, data)
            .await
            .map_err(BlobStoreError::Io)?;
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

    async fn put_chunked(&self, hash: &ContentHash, data: &[u8]) -> BlobStoreResult<()> {
        let chunk_dir = self.chunk_dir(hash);
        tokio_fs::create_dir_all(&chunk_dir)
            .await
            .map_err(BlobStoreError::Io)?;

        let mut chunk_hashes = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            let chunk_size = (data.len() - offset).min(CHUNK_THRESHOLD);
            let chunk = &data[offset..offset + chunk_size];
            let chunk_hash = ContentHash::new(chunk);
            let chunk_path = chunk_dir.join(chunk_hash.to_hex());

            if !chunk_path.exists() {
                self.write_file(&chunk_path, chunk).await?;
            }
            chunk_hashes.push(chunk_hash);
            offset += chunk_size;
        }

        let manifest = serde_json::to_vec(&chunk_hashes).map_err(|e| {
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

        let manifest_data = self.read_file(&manifest_path).await?;
        let chunk_hashes: Vec<ContentHash> =
            serde_json::from_slice(&manifest_data).map_err(|e| {
                BlobStoreError::Storage(format!("Failed to parse chunk manifest: {}", e))
            })?;

        let mut result = Vec::with_capacity(chunk_hashes.len() * CHUNK_THRESHOLD);
        for chunk_hash in chunk_hashes {
            let chunk_path = self.chunk_dir(hash).join(chunk_hash.to_hex());
            let chunk_data = self.read_file(&chunk_path).await?;
            result.extend_from_slice(&chunk_data);
        }

        Ok(Some(Bytes::from(result)))
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
            return Ok(Some(decode_blob(&framed)?));
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
            self.put_chunked(&hash, data).await?;
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
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".manifest") {
                    continue;
                }
                if !name.starts_with(prefix_str) {
                    continue;
                }
                if let Ok(hash) = ContentHash::from_hex(&name) {
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
}
