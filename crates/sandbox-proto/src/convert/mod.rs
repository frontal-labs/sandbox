//! Conversions between `sandbox-types` domain values and proto messages.
//!
//! Domain to proto is infallible. Proto to domain validates everything and
//! returns `SandboxError` with `ErrorCode::InvalidArgument` on bad input, so a
//! malformed request never reaches the domain code as a half-built value.

use sandbox_types::{ErrorCode, SandboxError};

pub mod error;
pub mod event;
pub mod exec;
pub mod hash;
pub mod ids;
pub mod policy;
pub mod session;
pub mod time;

pub(crate) fn invalid(message: impl Into<String>) -> SandboxError {
    SandboxError::new(ErrorCode::InvalidArgument, message)
}
