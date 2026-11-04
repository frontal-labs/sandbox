//! `sandbox_types::ErrorCode` and its proto enum. Explicit on both sides, so
//! adding a variant fails to compile until the proto and this table agree.

use super::invalid;
use crate::v1::types as pb;
use sandbox_types::{ErrorCode, SandboxError};

impl From<ErrorCode> for pb::ErrorCode {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::Internal => Self::Internal,
            ErrorCode::InvalidArgument => Self::InvalidArgument,
            ErrorCode::NotFound => Self::NotFound,
            ErrorCode::AlreadyExists => Self::AlreadyExists,
            ErrorCode::PermissionDenied => Self::PermissionDenied,
            ErrorCode::ResourceExhausted => Self::ResourceExhausted,
            ErrorCode::Unavailable => Self::Unavailable,
            ErrorCode::Timeout => Self::Timeout,
            ErrorCode::Cancelled => Self::Cancelled,
            ErrorCode::InvalidState => Self::InvalidState,
            ErrorCode::Unsupported => Self::Unsupported,
            ErrorCode::PolicyDenied => Self::PolicyDenied,
            ErrorCode::PromotionFailed => Self::PromotionFailed,
            ErrorCode::SnapshotFailed => Self::SnapshotFailed,
            ErrorCode::VmBootFailed => Self::VmBootFailed,
            ErrorCode::NetworkError => Self::NetworkError,
            ErrorCode::CasError => Self::CasError,
            ErrorCode::VfsError => Self::VfsError,
            ErrorCode::ShellError => Self::ShellError,
            ErrorCode::RuntimeError => Self::RuntimeError,
            ErrorCode::WasmError => Self::WasmError,
            ErrorCode::NeedsVm => Self::NeedsVm,
        }
    }
}

impl TryFrom<pb::ErrorCode> for ErrorCode {
    type Error = SandboxError;
    fn try_from(code: pb::ErrorCode) -> Result<Self, Self::Error> {
        Ok(match code {
            pb::ErrorCode::Unspecified => return Err(invalid("error code is unspecified")),
            pb::ErrorCode::Internal => Self::Internal,
            pb::ErrorCode::InvalidArgument => Self::InvalidArgument,
            pb::ErrorCode::NotFound => Self::NotFound,
            pb::ErrorCode::AlreadyExists => Self::AlreadyExists,
            pb::ErrorCode::PermissionDenied => Self::PermissionDenied,
            pb::ErrorCode::ResourceExhausted => Self::ResourceExhausted,
            pb::ErrorCode::Unavailable => Self::Unavailable,
            pb::ErrorCode::Timeout => Self::Timeout,
            pb::ErrorCode::Cancelled => Self::Cancelled,
            pb::ErrorCode::InvalidState => Self::InvalidState,
            pb::ErrorCode::Unsupported => Self::Unsupported,
            pb::ErrorCode::PolicyDenied => Self::PolicyDenied,
            pb::ErrorCode::PromotionFailed => Self::PromotionFailed,
            pb::ErrorCode::SnapshotFailed => Self::SnapshotFailed,
            pb::ErrorCode::VmBootFailed => Self::VmBootFailed,
            pb::ErrorCode::NetworkError => Self::NetworkError,
            pb::ErrorCode::CasError => Self::CasError,
            pb::ErrorCode::VfsError => Self::VfsError,
            pb::ErrorCode::ShellError => Self::ShellError,
            pb::ErrorCode::RuntimeError => Self::RuntimeError,
            pb::ErrorCode::WasmError => Self::WasmError,
            pb::ErrorCode::NeedsVm => Self::NeedsVm,
        })
    }
}
