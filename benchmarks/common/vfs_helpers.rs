use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_vfs::{Vfs, VfsConfig, VfsOptions};
use std::path::PathBuf;

pub struct VfsBenchHarness {
    pub vfs: Vfs,
    pub ctx: BenchmarkContext,
    pub test_files: Vec<PathBuf>,
}

impl VfsBenchHarness {
    pub async fn new(ctx: &BenchmarkContext) -> Result<Self> {
        let config = VfsConfig::default();
        let vfs = Vfs::new(config).await?;
        
        Ok(Self {
            vfs,
            ctx: BenchmarkContext::new()?,
            test_files: Vec::new(),
        })
    }

    pub async fn create_test_file(&mut self, path: &str, content: &[u8]) -> Result<PathBuf> {
        let full_path = self.ctx.temp_dir.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, content).await?;
        self.test_files.push(full_path.clone());
        Ok(full_path)
    }

    pub async fn setup_large_dir(&mut self, file_count: usize, file_size: usize) -> Result<()> {
        let dir = self.ctx.temp_dir.join("large_dir");
        tokio::fs::create_dir_all(&dir).await?;
        
        let content = vec![0u8; file_size];
        for i in 0..file_count {
            let file_path = dir.join(format!("file_{:06}.dat", i));
            tokio::fs::write(&file_path, &content).await?;
            self.test_files.push(file_path);
        }
        Ok(())
    }

    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }
}

pub fn vfs_config_bench() -> VfsConfig {
    VfsConfig::default()
}