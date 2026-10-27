use async_trait::async_trait;
use bytes::Bytes;
use sandbox_types::hash::ContentHash;
use std::path::Path;

#[async_trait]
pub trait SessionFs: Send + Sync {
    /// Bytes of file content visible to the session, after quotas' exemptions.
    async fn total_bytes(&self) -> VfsResult<u64>;
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
    /// Starts receiving changes under `path`. See ADR 0005.
    async fn watch(&self, path: &Path) -> VfsResult<Watch>;
    /// Releases a handle. Using it afterwards fails with `InvalidPath`.
    fn close(&self, handle: &FileHandle);
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

/// A change under a watched path. Paths are normalized absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Created(String),
    Modified(String),
    Removed(String),
    Renamed {
        from: String,
        to: String,
    },
    /// Events were dropped because the reader fell behind. Re-read the tree.
    Overflow,
}

/// A subscription to changes under one path. Dropping it ends the subscription.
#[derive(Debug)]
pub struct Watch {
    prefix: String,
    rx: tokio::sync::broadcast::Receiver<WatchEvent>,
}

impl Watch {
    pub(crate) fn new(prefix: String, rx: tokio::sync::broadcast::Receiver<WatchEvent>) -> Self {
        Self { prefix, rx }
    }

    /// Waits for the next event under the watched path. Returns `None` once the
    /// VFS that produced the events has been dropped.
    pub async fn next(&mut self) -> Option<WatchEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if self.covers(&event) {
                        return Some(event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    return Some(WatchEvent::Overflow);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    fn covers(&self, event: &WatchEvent) -> bool {
        match event {
            WatchEvent::Created(p) | WatchEvent::Modified(p) | WatchEvent::Removed(p) => {
                is_within(p, &self.prefix)
            }
            WatchEvent::Renamed { from, to } => {
                is_within(from, &self.prefix) || is_within(to, &self.prefix)
            }
            WatchEvent::Overflow => true,
        }
    }
}

/// True when `path` is `prefix` or lies below it.
pub(crate) fn is_within(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

use crate::error::VfsError;
pub type VfsResult<T> = Result<T, VfsError>;
