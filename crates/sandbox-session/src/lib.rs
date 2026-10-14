//! Session trait and isolate backend for the sandbox system

pub mod backend;
pub mod session;

pub use backend::{restore, IsolateBackend};
pub use session::Session;
