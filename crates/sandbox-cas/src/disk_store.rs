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
use tracing::{debug, trace, warn};

const SHARD_BITS: u8 = 4;
pub const SHARD_COUNT: usize = 1 << SHARD_BITS;
pub const CHUNK_THRESHOLD: usize = 64 * 1024;
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
/// Pattern for temporary files created by write_file.
const TEMP_PATTERN: &str = ".tmp-";

#[derive(Debug)]
pub struct DiskStore {
    pub root: PathBuf,
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

        let store = Self {
            root: root.clone(),
            shards: Arc::new(shards),
        };

        // Clean up stale temporary files from crashed writes (ADR 0017).
        if let Err(e) = store.cleanup_stale_temps() {
            warn!("Failed to clean up stale temporary files: {}", e);
        }

        Ok(store)
    }

    /// Removes temporary files left behind by crashed writes.
    /// These files match the pattern `.tmp-<pid>-<seq>`.
    fn cleanup_stale_temps(&self) -> BlobStoreResult<()> {
        let shard_dirs = (0..SHARD_COUNT)
            .map(|i| self.root.join(format!("{:02x}", i)))
            .filter(|dir| dir.exists());
        for shard_dir in shard_dirs {
            let entries = fs::read_dir(&shard_dir).map_err(BlobStoreError::Io)?;
            for entry in entries {
                let entry = entry.map_err(BlobStoreError::Io)?;
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if name.contains(TEMP_PATTERN) {
                    let temp_path = entry.path();
                    if let Err(e) = fs::remove_file(&temp_path) {
                        warn!(
                            "Failed to remove stale temp file {}: {}",
                            temp_path.display(),
                            e
                        );
                    } else {
                        trace!("Removed stale temp file: {}", temp_path.display());
                    }
                }
            }
        }
        Ok(())
    }

    fn shard_index(&self, hash: &ContentHash) -> usize {
        let bytes = hash.as_bytes();
        let first_byte = bytes[0];
        (first_byte as usize) & (SHARD_COUNT - 1)
    }

    pub fn shard_path(&self, hash: &ContentHash) -> PathBuf {
        let idx = self.shard_index(hash);
        self.root.join(format!("{:02x}", idx)).join(hash.to_hex())
    }

    /// The directory of a version 1 blob's chunks. Version 1 keyed chunks by the
    /// blob that wrote them, so identical chunks were stored once per blob.
    pub fn chunk_dir(&self, hash: &ContentHash) -> PathBuf {
        let idx = self.shard_index(hash);
        self.root
            .join(format!("{:02x}", idx))
            .join("chunks")
            .join(&hash.to_hex()[..2])
    }

    /// The file of a chunk in the store-wide chunk area. A chunk is named by its
    /// own hash, so identical chunks from different blobs are one file (ADR 0017).
    pub fn chunk_file(&self, chunk: &ContentHash) -> PathBuf {
        self.chunk_dir(chunk).join(chunk.to_hex())
    }

    /// Writes `data` to `path` through a temporary file and a rename, so a reader
    /// never sees a partly written file, and a crash leaves either the old file
    /// or the new one. Chunks are shared between blobs, so a truncated chunk
    /// would corrupt every blob that reuses it.
    pub async fn write_file(&self, path: &Path, data: &[u8]) -> BlobStoreResult<()> {
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
pub fn encode_blob(data: &[u8], compress: bool) -> BlobStoreResult<Vec<u8>> {
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

pub fn decode_blob(framed: &[u8]) -> BlobStoreResult<Bytes> {
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
        let chunk_dirs = (0..SHARD_COUNT)
            .map(|i| (i, self.root.join(format!("{:02x}", i)).join("chunks")))
            .filter(|(_, chunks)| chunks.exists());
        for (i, chunks) in chunk_dirs {
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
                    if ContentHash::from_hex(&name).is_ok() {
                        keys.push(format!(
                            "{:02x}/chunks/{}/{}",
                            i,
                            prefix.file_name().to_string_lossy(),
                            name
                        ));
                    }
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
            if !keep.contains(&hash) {
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
        }
        Ok(removed)
    }

    async fn list(&self, prefix: Option<&str>) -> BlobStoreResult<Vec<ContentHash>> {
        let mut results = Vec::new();
        let prefix_str = prefix.unwrap_or("");

        let shard_dirs = (0..SHARD_COUNT)
            .map(|i| self.root.join(format!("{:02x}", i)))
            .filter(|dir| dir.exists());
        for shard_dir in shard_dirs {
            let mut entries = tokio_fs::read_dir(&shard_dir)
                .await
                .map_err(BlobStoreError::Io)?;
            while let Some(entry) = entries.next_entry().await.map_err(BlobStoreError::Io)? {
                // A chunked blob has a manifest and no file of its own, so the
                // manifest's name is the blob's hash.
                let name = entry.file_name().to_string_lossy().to_string();
                let hex = name.strip_suffix(".manifest").unwrap_or(&name);
                if hex.starts_with(prefix_str) {
                    if let Ok(hash) = ContentHash::from_hex(hex) {
                        results.push(hash);
                    }
                }
            }
        }

        Ok(results)
    }

    fn name(&self) -> &'static str {
        "disk"
    }
}
