//! WASI preview1 host for command modules, with files backed by the VFS.
//! See [`wasi`] for what the guest can and cannot reach.

pub mod registry;
pub mod wasi;

pub use registry::{Registry, ToolSpec};

pub use wasi::{
    is_cached, run_module, run_module_with_mount, run_module_with_mount_streamed, Mount,
    StreamObserver, WasiLimits, WasiOutcome,
};
