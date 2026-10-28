//! Rust SDK for the sandbox system. Every call goes to a running `sandbox-server`.

pub mod client;
pub mod embedded;

pub use client::{CheckpointResponse, CheckpointUsage, Client, ExecResult, ForkedChild};
pub use embedded::EmbeddedSandbox;
