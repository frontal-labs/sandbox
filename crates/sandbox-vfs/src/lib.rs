//! Virtual filesystem for the sandbox system

pub mod diff;
pub mod error;
pub mod node;
pub mod ops;
pub mod quota;
pub mod vfs;

pub use diff::{diff_trees, reachable_blobs, TreeChange};
pub use error::{VfsError, VfsResult};
pub use ops::*;
pub use quota::Quotas;
pub use vfs::{Vfs, SYSTEM_FILE_MAX_BYTES};
