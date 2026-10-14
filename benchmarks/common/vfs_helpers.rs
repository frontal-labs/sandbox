use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::path::PathBuf;
use std::sync::Arc;

pub struct VfsBenchHarness {
    pub vfs: Vfs,
    pub ctx: BenchmarkContext,
    pub test_files: Vec<PathBuf>,
}

impl VfsBenchHarness {
    /// An empty VFS over an in-memory blob store.
    pub fn new() -> Result<Self> {
        let store = Arc::new(MemStore::new());
        Ok(Self {
            vfs: Vfs::new(ContentHash::nil(), store),
            ctx: BenchmarkContext::new()?,
            test_files: Vec::new(),
        })
    }

    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }
}
