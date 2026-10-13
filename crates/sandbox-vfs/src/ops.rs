use sandbox_types::hash::ContentHash;
use async_trait::async_trait;
use bytes::Bytes;
use std::path::Path;

#[async_trait]
pub trait SessionFs: Send + Sync {
    async fn open(&self, path: &Path, flags: OpenFlags) -> VfsResult<FileHandle>;
    async fn read(&self, handle: &FileHandle, offset: u64, len: usize) -> VfsResult<Bytes>;
    async fn write(&self, handle: &FileHandle, offset: u64, data: &[u8]) -> VfsResult<usize>;
    async fn stat(&self, path: &Path) -> VfsResult<FileStat>;
    async fn readdir(&self, path: &Path) -> VfsResult<Vec<DirEntry>>;
    async fn mkdir(&self, path: &Path, mode: u32) -> VfsResult<()>;
    async fn rm(&self, path: &Path, recursive: bool) -> VfsResult<()>;
    async fn rename(&self, from: &Path, to: &Path) -> VfsResult<()>;
    async fn symlink(&self, target: &Path, link: &Path) -> VfsResult<()>;
    async fn readlink(&self, path: &Path) -> VfsResult<String>;
    async fn truncate(&self, path: &Path, size: u64) -> VfsResult<()>;
    async fn watch(&self, path: &Path) -> VfsResult<WatchHandle>;
    async fn commit(&mut self) -> VfsResult<ContentHash>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub create: bool,
    pub truncate: bool,
    pub exclusive: bool,
}

impl Default for OpenFlags {
    fn default() -> Self {
        Self {
            read: true,
            write: false,
            append: false,
            create: false,
            truncate: false,
            exclusive: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHandle(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mode: u32,
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchHandle(pub u64);

use crate::error::VfsError;
pub type VfsResult<T> = Result<T, VfsError>;