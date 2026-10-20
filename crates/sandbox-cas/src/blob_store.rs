use async_trait::async_trait;
use bytes::Bytes;
use sandbox_types::hash::ContentHash;
use sandbox_types::SandboxError;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PutOptions {
    pub compress: bool,
    pub chunk_if_large: bool,
}

impl Default for PutOptions {
    fn default() -> Self {
        Self {
            compress: true,
            chunk_if_large: true,
        }
    }
}

#[async_trait]
pub trait BlobStore: Send + Sync + fmt::Debug {
    async fn get(&self, hash: &ContentHash) -> BlobStoreResult<Option<Bytes>>;

    async fn put(&self, data: &[u8], options: PutOptions) -> BlobStoreResult<ContentHash>;

    async fn has(&self, hash: &ContentHash) -> BlobStoreResult<bool>;

    async fn delete(&self, hash: &ContentHash) -> BlobStoreResult<bool>;

    async fn list(&self, prefix: Option<&str>) -> BlobStoreResult<Vec<ContentHash>>;

    /// Chunk files kept for large blobs, as opaque keys. Chunks are shared between
    /// blobs, so a chunk is removed only by `delete_parts`, after the blobs that
    /// name it are gone. Stores that do not chunk list nothing.
    async fn list_parts(&self) -> BlobStoreResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// The chunk hashes a blob is stored as, in order. Empty for a blob that is
    /// stored whole.
    async fn parts(&self, _hash: &ContentHash) -> BlobStoreResult<Vec<ContentHash>> {
        Ok(Vec::new())
    }

    /// Removes the chunk files among `candidates` whose hash is not in `keep`.
    /// Returns how many were removed.
    async fn delete_parts(
        &self,
        _candidates: &[String],
        _keep: &HashSet<ContentHash>,
    ) -> BlobStoreResult<usize> {
        Ok(0)
    }

    fn name(&self) -> &'static str;
}

pub type BlobStoreResult<T> = Result<T, BlobStoreError>;

#[derive(Debug, thiserror::Error)]
pub enum BlobStoreError {
    #[error("Blob not found: {0}")]
    NotFound(ContentHash),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Chunking error: {0}")]
    Chunking(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

impl From<BlobStoreError> for SandboxError {
    fn from(err: BlobStoreError) -> Self {
        match err {
            BlobStoreError::NotFound(hash) => {
                SandboxError::not_found(format!("Blob not found: {}", hash))
            }
            BlobStoreError::Io(e) => {
                SandboxError::with_source(sandbox_types::ErrorCode::CasError, "I/O error", e)
            }
            BlobStoreError::Compression(e) => {
                SandboxError::cas_error(format!("Compression error: {}", e))
            }
            BlobStoreError::Chunking(e) => {
                SandboxError::cas_error(format!("Chunking error: {}", e))
            }
            BlobStoreError::Storage(e) => SandboxError::cas_error(format!("Storage error: {}", e)),
            BlobStoreError::InvalidArgument(e) => SandboxError::invalid_argument(e),
        }
    }
}
