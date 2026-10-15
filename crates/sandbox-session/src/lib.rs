//! Session trait and isolate backend for the sandbox system

pub mod backend;
pub mod session;

pub use backend::{install_blob_store, restore, IsolateBackend};
pub use session::Session;
