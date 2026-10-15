//! Rust SDK for the sandbox system. Every call goes to a running `sandbox-server`.

pub mod client;
pub mod embedded;

pub use client::{Client, ExecResult};
pub use embedded::EmbeddedSandbox;
