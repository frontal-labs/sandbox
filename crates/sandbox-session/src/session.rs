//! Session trait and types

use async_trait::async_trait;
use sandbox_types::{ExecRequest, ExecResult, SandboxError, SessionId, SnapshotId, Tier};

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
    /// Parses and runs a shell script. Working directory and environment
    /// persist across calls.
    async fn run_script(&self, script: &str) -> Result<ExecResult, SandboxError>;
    fn fs(&self) -> &dyn SessionFs;
    async fn snapshot(&self) -> Result<SnapshotId, SandboxError>;
    async fn sleep(&self) -> Result<SnapshotId, SandboxError>;
    async fn destroy(self: Box<Self>) -> Result<(), SandboxError>;
}
