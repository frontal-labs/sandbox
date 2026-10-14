use sandbox_cas::blob_store::BlobStoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VfsError {
    #[error("Path not found: {0}")]
    NotFound(String),

    #[error("Path already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CAS error: {0}")]
    Cas(#[from] BlobStoreError),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unsupported: {0}")]
    Unsupported(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type VfsResult<T> = Result<T, VfsError>;
