#![cfg(unix)]
//! Worker processes for the sandbox system. Each worker runs one session in a
//! child process, served as gRPC over a private Unix socket. See [`worker`].

pub mod worker;

pub use worker::{serve, Worker, MAX_MESSAGE_BYTES};
