//! Session trait and isolate backend for the sandbox system

pub mod agent_env;
pub mod backend;
pub mod filesystem;
pub mod session;

pub use backend::{
    blob_store, changed_since, committed_root, install_blob_store, quotas_for, restore,
    restore_checkpoint, restore_checkpoint_limited, restore_limited, restore_session_limited,
    IsolateBackend,
};
pub use filesystem::{delete_file, replay_run, vfs_error, write_file, write_path};
pub use sandbox_shell::{OutputChunk, OutputObserver};
pub use session::Checkpoint;
pub use session::Session;
