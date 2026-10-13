//! Session trait and types

use sandbox_types::{ExecRequest, SandboxError, SessionId, SnapshotId, Tier};
use async_trait::async_trait;

#[async_trait]
pub trait Session: Send + Sync {
    fn id(&self) -> SessionId;
    fn tier(&self) -> Tier;
    async fn exec(&self, req: ExecRequest) -> Result<ExecHandle, SandboxError>;
    fn fs(&self) -> Box<dyn SessionFs>;
    async fn snapshot(&self) -> Result<SnapshotId, SandboxError>;
    async fn sleep(&self) -> Result<SnapshotId, SandboxError>;
    async fn destroy(self: Box<Self>) -> Result<(), SandboxError>;
}

#[async_trait]
pub trait SessionFs: Send + Sync {
    async fn open(&self, _path: &std::path::Path, _flags: u32) -> Result<u64, SandboxError>;
    async fn read(&self, _handle: u64, _offset: u64, _len: usize) -> Result<Vec<u8>, SandboxError>;
    async fn write(&self, _handle: u64, _offset: u64, _data: &[u8]) -> Result<usize, SandboxError>;
    async fn stat(&self, _path: &std::path::Path) -> Result<FileStat, SandboxError>;
    async fn readdir(&self, _path: &std::path::Path) -> Result<Vec<DirEntry>, SandboxError>;
    async fn mkdir(&self, _path: &std::path::Path, _mode: u32) -> Result<(), SandboxError>;
    async fn rm(&self, _path: &std::path::Path, _recursive: bool) -> Result<(), SandboxError>;
    async fn rename(&self, _from: &std::path::Path, _to: &std::path::Path) -> Result<(), SandboxError>;
    async fn symlink(&self, _target: &std::path::Path, _link: &std::path::Path) -> Result<(), SandboxError>;
    async fn readlink(&self, _path: &std::path::Path) -> Result<String, SandboxError>;
    async fn truncate(&self, _path: &std::path::Path, _size: u64) -> Result<(), SandboxError>;
    async fn commit(&self) -> Result<sandbox_types::hash::ContentHash, SandboxError>;
}

#[derive(Debug, Default, Clone)]
pub struct FileStat {
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mode: u32,
    pub modified: u64,
}

#[derive(Debug, Default, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mode: u32,
}

#[derive(Debug, Default)]
pub struct ExecHandle {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdin: Vec<u8>,
}

impl ExecHandle {
    pub async fn wait(self) -> Result<sandbox_types::ExecResult, SandboxError> {
        Ok(sandbox_types::ExecResult {
            exit_code: 0,
            stdout: self.stdout,
            stderr: self.stderr,
            cpu_ms: 0,
            wall_ms: 0,
        })
    }
}