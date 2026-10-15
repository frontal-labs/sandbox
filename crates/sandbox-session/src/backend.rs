use crate::session::{Session, SessionFs};
use async_trait::async_trait;
use sandbox_cas::{BlobStore, MemStore};
use sandbox_shell::{ExecutionResult, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig, SessionId, SnapshotId, Tier,
};
use sandbox_vfs::Vfs;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::Mutex;

/// Blob store shared by every backend in the process. Blobs are content
/// addressed and never change, so a restored session can read the blobs of
/// the snapshot it came from. Nothing is freed yet: a process-wide store only
/// grows.
fn shared_store() -> Arc<dyn BlobStore> {
    static STORE: OnceLock<Arc<dyn BlobStore>> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(MemStore::new())).clone()
}

/// Tier 1 session. Owns its VFS, which is the source of truth for files, and
/// a shell that runs against that VFS.
///
/// A snapshot records the committed filesystem root. Sleep is not implemented
/// yet and returns `Unsupported`.
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
        Ok(Self::with_root(ContentHash::nil()))
    }

    /// A session whose filesystem starts at `root`, a committed snapshot root.
    fn with_root(root: ContentHash) -> Self {
        let fs = Arc::new(Vfs::new(root, shared_store()));
        let shell = Shell::new(fs.clone(), ShellConfig::default());
        Self {
            id: SessionId::new(),
            tier: Tier::Isolate,
            fs,
            shell: Mutex::new(shell),
        }
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

    async fn snapshot(&self) -> Result<ContentHash, SandboxError> {
        self.fs
            .checkpoint()
            .await
            .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))
    }

    async fn sleep(&self) -> Result<SnapshotId, SandboxError> {
        Err(SandboxError::unsupported("sleep is not implemented yet"))
    }

    async fn destroy(self: Box<Self>) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// Starts a new session whose filesystem is `root`, a committed root returned
/// by `Session::snapshot`. The caller is responsible for who may use `root`.
pub async fn restore(root: ContentHash) -> Result<Box<dyn Session>, SandboxError> {
    Ok(Box::new(IsolateBackend::with_root(root)))
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

    async fn write_text(fs: &dyn SessionFs, path: &str, text: &str) {
        let handle = fs
            .open(
                Path::new(path),
                OpenFlags {
                    read: false,
                    write: true,
                    append: false,
                    create: true,
                    truncate: true,
                    exclusive: false,
                },
            )
            .await
            .unwrap();
        fs.write(&handle, 0, text.as_bytes()).await.unwrap();
        fs.close(&handle);
    }

    async fn read_text(fs: &dyn SessionFs, path: &str) -> String {
        let handle = fs
            .open(Path::new(path), OpenFlags::default())
            .await
            .unwrap();
        let bytes = fs.read(&handle, 0, 1024).await.unwrap();
        fs.close(&handle);
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn sleep_reports_unsupported() {
        let backend = new_backend().await;
        assert_eq!(
            backend.sleep().await.err().unwrap().code,
            ErrorCode::Unsupported
        );
    }

    #[tokio::test]
    async fn restored_session_sees_the_snapshot_not_later_writes() {
        let backend = new_backend().await;
        write_text(backend.fs(), "/a.txt", "one").await;
        let root = backend.snapshot().await.unwrap();

        // Changes after the snapshot must not reach the restored session.
        write_text(backend.fs(), "/a.txt", "two").await;
        write_text(backend.fs(), "/later.txt", "new").await;

        let restored = restore(root).await.unwrap();
        assert_eq!(read_text(restored.fs(), "/a.txt").await, "one");
        assert!(restored.fs().stat(Path::new("/later.txt")).await.is_err());

        // The original keeps its own later state.
        assert_eq!(read_text(backend.fs(), "/a.txt").await, "two");
    }
}
