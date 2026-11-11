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
    /// Like `run_script`, with a top-level `cat` reading `input` as it arrives, and output
    /// reported to `observer` as it is produced (ADR 0046). A tier that cannot stream input refuses.
    async fn run_script_live(
        &self,
        script: &str,
        input: sandbox_shell::LiveInput,
        observer: sandbox_shell::OutputObserver,
    ) -> Result<ExecResult, SandboxError> {
        let _ = (script, input, observer);
        Err(SandboxError::unsupported(
            "this session cannot stream input to a running command",
        ))
    }
    fn fs(&self) -> &dyn SessionFs;
    /// The generator state behind `$RANDOM`. A run records it at its start.
    async fn random_state(&self) -> u64;
    /// Sets the `$RANDOM` generator state, so a replayed run draws the same values.
    async fn set_random_state(&self, state: u64);
    /// Writes a host-owned file such as `/etc/agent/env.json`. The agent cannot
    /// write these paths, so only the host calls this.
    async fn write_system_file(&self, path: &str, data: &[u8]) -> Result<(), SandboxError>;
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
