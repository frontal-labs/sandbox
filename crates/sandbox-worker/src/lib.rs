#![cfg(unix)]
//! Worker processes for the sandbox system. Each worker runs one session in a
//! child process, served as gRPC over a private Unix socket. See [`worker`].
//! Contained `node` scripts also run in a child process; see [`script`].

pub mod isolation;
pub mod script;
pub mod worker;

pub use isolation::{enter, CgroupLimits, Isolation};
pub use script::{run_script_child, ScriptProcess, SCRIPT_RUN_SUBCOMMAND};
pub use worker::{serve, Worker, MAX_MESSAGE_BYTES};
