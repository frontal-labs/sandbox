//! Session trait and isolate backend for the sandbox system

pub mod backend;
pub mod session;

pub use backend::{install_blob_store, restore, restore_checkpoint, IsolateBackend};
pub use session::Checkpoint;
pub use session::Session;
