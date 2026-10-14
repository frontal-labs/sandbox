//! Worker processes for the sandbox system. Each worker runs its session in a
//! child process. See [`worker`] for the protocol and what is isolated.

pub mod worker;

pub use worker::{run_stdio_worker, serve, Request, Response, Worker};
