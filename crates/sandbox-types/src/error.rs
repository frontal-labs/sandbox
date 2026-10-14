use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Internal,
    InvalidArgument,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    Unavailable,
    Timeout,
    Cancelled,
    InvalidState,
    Unsupported,
    PolicyDenied,
    PromotionFailed,
    SnapshotFailed,
    VmBootFailed,
    NetworkError,
    CasError,
    VfsError,
    ShellError,
    RuntimeError,
    WasmError,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Error)]
#[error("[{code}] {message}")]
pub struct SandboxError {
    pub code: ErrorCode,
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl SandboxError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AlreadyExists, message)
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }

    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ResourceExhausted, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Cancelled, message)
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidState, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn policy_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PolicyDenied, message)
    }

    pub fn promotion_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PromotionFailed, message)
    }

    pub fn snapshot_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::SnapshotFailed, message)
    }

    pub fn vm_boot_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::VmBootFailed, message)
    }

    pub fn network_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NetworkError, message)
    }

    pub fn cas_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CasError, message)
    }

    pub fn vfs_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::VfsError, message)
    }

    pub fn shell_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ShellError, message)
    }

    pub fn runtime_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RuntimeError, message)
    }

    pub fn wasm_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::WasmError, message)
    }
}

impl From<std::io::Error> for SandboxError {
    fn from(err: std::io::Error) -> Self {
        SandboxError::with_source(ErrorCode::Internal, "I/O error", err)
    }
}

impl From<blake3::HexError> for SandboxError {
    fn from(err: blake3::HexError) -> Self {
        SandboxError::with_source(ErrorCode::CasError, "BLAKE3 error", err)
    }
}

impl From<serde_json::Error> for SandboxError {
    fn from(err: serde_json::Error) -> Self {
        SandboxError::with_source(ErrorCode::Internal, "JSON error", err)
    }
}

pub type Result<T> = std::result::Result<T, SandboxError>;
