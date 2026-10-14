//! Maps `SandboxError` to `tonic::Status` and back.
//!
//! The exact `ErrorCode` travels in the metadata key [`CODE_METADATA_KEY`], so
//! the round trip keeps codes that gRPC's own statuses cannot express (for
//! example `ShellError`). Clients that ignore the key still get a sensible
//! gRPC code.

use sandbox_types::{ErrorCode, SandboxError};
use tonic::{Code, Status};

pub const CODE_METADATA_KEY: &str = "sandbox-error-code";

/// The gRPC status code that best matches a sandbox error code.
pub fn grpc_code(code: ErrorCode) -> Code {
    match code {
        ErrorCode::InvalidArgument => Code::InvalidArgument,
        ErrorCode::NotFound => Code::NotFound,
        ErrorCode::AlreadyExists => Code::AlreadyExists,
        ErrorCode::PermissionDenied | ErrorCode::PolicyDenied => Code::PermissionDenied,
        ErrorCode::ResourceExhausted => Code::ResourceExhausted,
        ErrorCode::Unavailable => Code::Unavailable,
        ErrorCode::Timeout => Code::DeadlineExceeded,
        ErrorCode::Cancelled => Code::Cancelled,
        ErrorCode::InvalidState => Code::FailedPrecondition,
        ErrorCode::Unsupported => Code::Unimplemented,
        _ => Code::Internal,
    }
}

/// Converts a sandbox error to a status, attaching the exact code as metadata.
pub fn to_status(error: &SandboxError) -> Status {
    let mut status = Status::new(grpc_code(error.code), error.message.clone());
    let name = format!("{:?}", error.code);
    if let Ok(value) = name.parse() {
        status.metadata_mut().insert(CODE_METADATA_KEY, value);
    }
    status
}

/// Converts a status back to a sandbox error. Uses the metadata code when it
/// is present and valid, and otherwise falls back to the gRPC code.
pub fn from_status(status: &Status) -> SandboxError {
    let code = status
        .metadata()
        .get(CODE_METADATA_KEY)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_error_code)
        .unwrap_or_else(|| fallback_code(status.code()));
    SandboxError::new(code, status.message().to_string())
}

fn fallback_code(code: Code) -> ErrorCode {
    match code {
        Code::InvalidArgument => ErrorCode::InvalidArgument,
        Code::NotFound => ErrorCode::NotFound,
        Code::AlreadyExists => ErrorCode::AlreadyExists,
        Code::PermissionDenied => ErrorCode::PermissionDenied,
        Code::ResourceExhausted => ErrorCode::ResourceExhausted,
        Code::Unavailable => ErrorCode::Unavailable,
        Code::DeadlineExceeded => ErrorCode::Timeout,
        Code::Cancelled => ErrorCode::Cancelled,
        Code::FailedPrecondition => ErrorCode::InvalidState,
        Code::Unimplemented => ErrorCode::Unsupported,
        _ => ErrorCode::Internal,
    }
}

/// Parses the `Debug` name of an `ErrorCode`. Every variant is listed, so a new
/// variant fails the `error_code_names_round_trip` test until it is added here.
pub fn parse_error_code(name: &str) -> Option<ErrorCode> {
    ALL_ERROR_CODES
        .iter()
        .copied()
        .find(|code| format!("{code:?}") == name)
}

/// Every sandbox error code, in declaration order.
pub const ALL_ERROR_CODES: &[ErrorCode] = &[
    ErrorCode::Internal,
    ErrorCode::InvalidArgument,
    ErrorCode::NotFound,
    ErrorCode::AlreadyExists,
    ErrorCode::PermissionDenied,
    ErrorCode::ResourceExhausted,
    ErrorCode::Unavailable,
    ErrorCode::Timeout,
    ErrorCode::Cancelled,
    ErrorCode::InvalidState,
    ErrorCode::Unsupported,
    ErrorCode::PolicyDenied,
    ErrorCode::PromotionFailed,
    ErrorCode::SnapshotFailed,
    ErrorCode::VmBootFailed,
    ErrorCode::NetworkError,
    ErrorCode::CasError,
    ErrorCode::VfsError,
    ErrorCode::ShellError,
    ErrorCode::RuntimeError,
    ErrorCode::WasmError,
];
