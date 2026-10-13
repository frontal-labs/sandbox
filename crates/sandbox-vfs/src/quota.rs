use sandbox_types::hash::ContentHash;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quotas {
    pub total_bytes: u64,
    pub inode_count: u64,
    pub max_file_size: u64,
    pub max_path_depth: u32,
}

impl Default for Quotas {
    fn default() -> Self {
        Self {
            total_bytes: 1024 * 1024 * 1024,
            inode_count: 100000,
            max_file_size: 100 * 1024 * 1024,
            max_path_depth: 256,
        }
    }
}

#[derive(Debug, Default)]
pub struct Usage {
    pub total_bytes: u64,
    pub inode_count: u64,
    pub file_sizes: HashMap<ContentHash, u64>,
}

impl Usage {
    pub fn add_file(&mut self, hash: ContentHash, size: u64) -> Result<(), QuotaExceeded> {
        if self.total_bytes + size > self.max_total_bytes() {
            return Err(QuotaExceeded::TotalBytes);
        }
        if self.inode_count + 1 > self.max_inodes() {
            return Err(QuotaExceeded::InodeCount);
        }
        if size > self.max_file_size() {
            return Err(QuotaExceeded::FileSize);
        }

        self.total_bytes += size;
        self.inode_count += 1;
        self.file_sizes.insert(hash, size);
        Ok(())
    }

    pub fn remove_file(&mut self, hash: &ContentHash) {
        if let Some(size) = self.file_sizes.remove(hash) {
            self.total_bytes = self.total_bytes.saturating_sub(size);
            self.inode_count = self.inode_count.saturating_sub(1);
        }
    }

    fn max_total_bytes(&self) -> u64 {
        1024 * 1024 * 1024
    }

    fn max_inodes(&self) -> u64 {
        100000
    }

    fn max_file_size(&self) -> u64 {
        100 * 1024 * 1024
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QuotaExceeded {
    #[error("Total bytes quota exceeded")]
    TotalBytes,
    #[error("Inode count quota exceeded")]
    InodeCount,
    #[error("File size quota exceeded")]
    FileSize,
    #[error("Path depth quota exceeded")]
    PathDepth,
}