use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_cas::{BlobStore, MemStore, DiskStore, TieredStore};
use std::path::PathBuf;
use tempfile::TempDir;

pub struct CasBenchHarness {
    pub store: Box<dyn BlobStore>,
    pub ctx: BenchmarkContext,
    _temp_dir: Option<TempDir>,
}

impl CasBenchHarness {
    pub async fn new_mem(ctx: &BenchmarkContext) -> Result<Self> {
        let store = Box::new(MemStore::new());
        Ok(Self {
            store,
            ctx: BenchmarkContext::new()?,
            _temp_dir: None,
        })
    }

    pub async fn new_disk(ctx: &BenchmarkContext) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let store = Box::new(DiskStore::new(temp_dir.path()).await?);
        Ok(Self {
            store,
            ctx: BenchmarkContext::new()?,
            _temp_dir: Some(temp_dir),
        })
    }

    pub async fn new_tiered(ctx: &BenchmarkContext, local_path: PathBuf) -> Result<Self> {
        let local = DiskStore::new(local_path).await?;
        let remote = MemStore::new();
        let store = Box::new(TieredStore::new(local, remote));
        Ok(Self {
            store,
            ctx: BenchmarkContext::new()?,
            _temp_dir: None,
        })
    }

    pub fn store(&self) -> &dyn BlobStore {
        &*self.store
    }

    pub fn generate_data(size: usize) -> Vec<u8> {
        use rand::RngCore;
        let mut data = vec![0u8; size];
        rand::rng().fill_bytes(&mut data);
        data
    }
}