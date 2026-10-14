use async_trait::async_trait;
use bytes::Bytes;
use sandbox_types::hash::ContentHash;
use sandbox_types::SandboxError;
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
