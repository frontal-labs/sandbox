use crate::session::{Session, SessionFs};
use async_trait::async_trait;
use sandbox_cas::{BlobStore, MemStore};
use sandbox_shell::{ExecutionResult, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig, SessionId, SnapshotId, Tier,
};
use sandbox_vfs::Vfs;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Tier 1 session. Owns its VFS, which is the source of truth for files, and
/// a shell that runs against that VFS.
///
/// Snapshot and sleep are not implemented yet and return `Unsupported`.
pub struct IsolateBackend {
    id: SessionId,
    tier: Tier,
    fs: Arc<Vfs>,
    /// Serializes commands, so the working directory and environment change
    /// in one order.
    shell: Mutex<Shell>,
}

impl IsolateBackend {
    pub async fn new(_config: SessionConfig) -> Result<Self, SandboxError> {
        let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
        let fs = Arc::new(Vfs::new(ContentHash::nil(), store));
        let shell = Shell::new(fs.clone(), ShellConfig::default());
        Ok(Self {
            id: SessionId::new(),
            tier: Tier::Isolate,
            fs,
            shell: Mutex::new(shell),
        })
    }
}

fn to_result(out: ExecutionResult, started: Instant) -> ExecResult {
    ExecResult {
        exit_code: out.exit_code,
        stdout: out.stdout,
        stderr: out.stderr,
        // CPU accounting is not implemented; this is wall time only.
        cpu_ms: 0,
        wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

#[async_trait]
impl Session for IsolateBackend {
    fn id(&self) -> SessionId {
        self.id
    }

    fn tier(&self) -> Tier {
        self.tier
    }

    async fn exec(&self, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        if req.stream {
            return Err(SandboxError::unsupported(
                "streaming exec is not implemented yet",
            ));
        }
        if req.timeout_ms.is_some() {
            // Refuse rather than silently ignore a limit the caller asked for.
            return Err(SandboxError::unsupported(
                "exec timeouts are not enforced yet",
            ));
        }
        if req.argv.is_empty() {
            return Err(SandboxError::new(
                ErrorCode::InvalidArgument,
                "argv is empty",
            ));
        }

        let started = Instant::now();
        let mut shell = self.shell.lock().await;
        for (key, value) in req.env {
            shell.set_env(key, value);
        }
        if let Some(cwd) = &req.cwd {
            shell.chdir(cwd).await?;
        }
        let stdin = req.stdin.unwrap_or_default();
        let out = shell.run_argv(&req.argv, &stdin).await?;
        Ok(to_result(out, started))
    }

    async fn run_script(&self, script: &str) -> Result<ExecResult, SandboxError> {
        let started = Instant::now();
        let mut shell = self.shell.lock().await;
        let out = shell.run(script).await?;
        Ok(to_result(out, started))
    }

    fn fs(&self) -> &dyn SessionFs {
        self.fs.as_ref()
    }

    async fn snapshot(&self) -> Result<SnapshotId, SandboxError> {
        Err(SandboxError::unsupported("snapshot is not implemented yet"))
    }

    async fn sleep(&self) -> Result<SnapshotId, SandboxError> {
        Err(SandboxError::unsupported("sleep is not implemented yet"))
    }

    async fn destroy(self: Box<Self>) -> Result<(), SandboxError> {
        Ok(())
    }
}

pub async fn restore(_snapshot_id: SnapshotId) -> Result<Box<dyn Session>, SandboxError> {
    Err(SandboxError::unsupported("restore not implemented"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_types::TenantId;
    use sandbox_vfs::OpenFlags;
    use std::collections::HashMap;
    use std::path::Path;

    async fn new_backend() -> IsolateBackend {
        let config = SessionConfig {
            tenant_id: TenantId::new(),
            ..SessionConfig::default()
        };
        IsolateBackend::new(config).await.unwrap()
    }

    fn argv(words: &[&str]) -> ExecRequest {
        ExecRequest {
            argv: words.iter().map(|w| w.to_string()).collect(),
            ..ExecRequest::default()
        }
    }

    #[tokio::test]
    async fn fs_writes_and_reads_back() {
        let backend = new_backend().await;
        let rw = OpenFlags {
            read: true,
            write: true,
            create: true,
            ..OpenFlags::default()
        };
        let fs = backend.fs();
        let h = fs.open(Path::new("/note.txt"), rw).await.unwrap();
        fs.write(&h, 0, b"kept").await.unwrap();
        let read = fs
            .open(Path::new("/note.txt"), OpenFlags::default())
            .await
            .unwrap();
        assert_eq!(fs.read(&read, 0, 16).await.unwrap().as_ref(), b"kept");
    }

    #[tokio::test]
    async fn exec_runs_a_command_and_returns_its_exit_code() {
        let backend = new_backend().await;
        let out = backend.exec(argv(&["echo", "hello"])).await.unwrap();
        assert_eq!(
            (out.exit_code, out.stdout.as_slice()),
            (0, b"hello\n".as_slice())
        );

        let missing = backend.exec(argv(&["nonexistent-tool"])).await.unwrap();
        assert_eq!(missing.exit_code, 127);
    }

    #[tokio::test]
    async fn run_script_keeps_state_between_calls() {
        let backend = new_backend().await;
        backend
            .run_script("mkdir -p /work && cd /work")
            .await
            .unwrap();
        let out = backend.run_script("pwd && echo x > f").await.unwrap();
        assert_eq!(out.stdout, b"/work\n");
        // The file written by the first script is visible through the fs API.
        let h = backend
            .fs()
            .open(Path::new("/work/f"), OpenFlags::default())
            .await
            .unwrap();
        assert_eq!(backend.fs().read(&h, 0, 16).await.unwrap().as_ref(), b"x\n");
    }

    #[tokio::test]
    async fn exec_applies_env_and_cwd_overrides() {
        let backend = new_backend().await;
        backend.run_script("mkdir /job").await.unwrap();
        let req = ExecRequest {
            argv: vec!["pwd".to_string()],
            env: HashMap::from([("TASK".to_string(), "1".to_string())]),
            cwd: Some("/job".to_string()),
            ..ExecRequest::default()
        };
        assert_eq!(backend.exec(req).await.unwrap().stdout, b"/job\n");
    }

    #[tokio::test]
    async fn unsupported_options_are_refused_not_ignored() {
        let backend = new_backend().await;
        let streamed = ExecRequest {
            stream: true,
            ..argv(&["true"])
        };
        assert_eq!(
            backend.exec(streamed).await.err().unwrap().code,
            ErrorCode::Unsupported
        );
        let timed = ExecRequest {
            timeout_ms: Some(10),
            ..argv(&["true"])
        };
        assert_eq!(
            backend.exec(timed).await.err().unwrap().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            backend
                .exec(ExecRequest::default())
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[tokio::test]
    async fn snapshot_and_sleep_report_unsupported() {
        let backend = new_backend().await;
        assert_eq!(
            backend.snapshot().await.err().unwrap().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            backend.sleep().await.err().unwrap().code,
            ErrorCode::Unsupported
        );
    }
}
