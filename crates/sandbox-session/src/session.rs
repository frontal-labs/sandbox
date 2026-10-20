//! Session trait and types

use async_trait::async_trait;
use sandbox_shell::OutputObserver;
use sandbox_types::hash::ContentHash;
use sandbox_types::{ExecRequest, ExecResult, SandboxError, SessionId, Tier};
use std::collections::HashMap;

/// Everything a session needs to come back: its committed filesystem root, its
/// working directory, and its environment. ADR 0010.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub root: ContentHash,
    pub cwd: String,
    pub env: HashMap<String, String>,
}

impl Checkpoint {
    /// A checkpoint with a filesystem root and the default shell state.
    pub fn root_only(root: ContentHash) -> Self {
        Self {
            root,
            cwd: "/".to_string(),
            env: HashMap::new(),
        }
    }
}

// The VFS is the source of truth for files (invariant 1), so sessions expose
// the VFS trait directly rather than a second, divergent definition.
pub use sandbox_vfs::SessionFs;

#[async_trait]
pub trait Session: Send + Sync {
    fn id(&self) -> SessionId;
    fn tier(&self) -> Tier;
    /// Runs `req.argv` as one command. Output is buffered and returned with
    /// the exit code.
    async fn exec(&self, req: ExecRequest) -> Result<ExecResult, SandboxError>;
    /// Like `exec`, and reports the command's output to `observer` as it becomes
    /// available. The returned result holds the same output. See `OutputObserver`
    /// for the granularity.
    async fn exec_streamed(
        &self,
        req: ExecRequest,
        observer: OutputObserver,
    ) -> Result<ExecResult, SandboxError>;
    /// Parses and runs a shell script. Working directory and environment
    /// persist across calls.
    async fn run_script(&self, script: &str) -> Result<ExecResult, SandboxError>;
    fn fs(&self) -> &dyn SessionFs;
    /// Writes the filesystem's committed root and returns its hash. The caller
    /// keeps the hash, and `restore` starts a session from it. The session
    /// keeps running. Take snapshots while the session is idle.
    async fn snapshot(&self) -> Result<ContentHash, SandboxError>;
    /// Commits the filesystem and captures the shell state. The session keeps
    /// running. Its owner saves the checkpoint to bring the session back later
    /// (ADR 0010). Sleep and durable sessions both use it.
    async fn checkpoint(&self) -> Result<Checkpoint, SandboxError>;
    async fn destroy(self: Box<Self>) -> Result<(), SandboxError>;
}
