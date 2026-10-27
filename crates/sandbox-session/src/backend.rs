use crate::session::{Checkpoint, Session, SessionFs};
use async_trait::async_trait;
use sandbox_cas::{BlobStore, MemStore};
use sandbox_shell::{ExecutionResult, OutputObserver, Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::policy::ResourceLimits;
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig, SessionId, Tier,
};
use sandbox_vfs::Vfs;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// The file the host writes into every Tier 1 session (IMAGE.md §4.5). It is exempt
/// from quotas, so a session's limits apply only to what the agent writes.
pub const ENV_JSON_PATH: &str = "/etc/agent/env.json";

/// The blob store shared by every backend in the process. It is set once, by
/// `install_blob_store`, or lazily to an in-memory store on first use.
static STORE: OnceLock<Arc<dyn BlobStore>> = OnceLock::new();

/// Blobs are content addressed and never change, so a restored session can read
/// the blobs of the snapshot it came from. Nothing is freed yet: a process-wide
/// store only grows.
fn shared_store() -> Arc<dyn BlobStore> {
    STORE.get_or_init(|| Arc::new(MemStore::new())).clone()
}

/// The blob store every session in this process uses. Garbage collection sweeps it.
pub fn blob_store() -> Arc<dyn BlobStore> {
    shared_store()
}

/// Makes `store` the blob store that every session uses, for example a disk
/// store that keeps sleeping sessions across restarts (ADR 0008). Call it before
/// the first session is created. It fails when a store is already in use, because
/// blobs written to one store would not be visible in another.
pub fn install_blob_store(store: Arc<dyn BlobStore>) -> Result<(), SandboxError> {
    STORE.set(store).map_err(|_| {
        SandboxError::new(
            ErrorCode::InvalidState,
            "a blob store is already in use; install it before the first session",
        )
    })
}

/// Tier 1 session. Owns its VFS, which is the source of truth for files, and
/// a shell that runs against that VFS.
///
/// A snapshot records the committed filesystem root. Sleep checkpoints the
/// same root, and the owner drops the session to release its memory.
pub struct IsolateBackend {
    id: SessionId,
    tier: Tier,
    fs: Arc<Vfs>,
    /// Serializes commands, so the working directory and environment change
    /// in one order.
    shell: Mutex<Shell>,
}

impl IsolateBackend {
    pub async fn new(config: SessionConfig) -> Result<Self, SandboxError> {
        Ok(Self::with_checkpoint(
            Checkpoint::root_only(ContentHash::nil()),
            &config.limits,
        ))
    }

    /// A session that starts from a checkpoint: its filesystem root, working
    /// directory, and environment.
    fn with_checkpoint(checkpoint: Checkpoint, limits: &ResourceLimits) -> Self {
        let fs = Arc::new(
            Vfs::new(checkpoint.root, shared_store())
                .with_quotas(quotas_for(limits))
                .with_exempt_files(&[ENV_JSON_PATH]),
        );
        let shell = Shell::new(
            fs.clone(),
            ShellConfig {
                cwd: checkpoint.cwd,
                env: checkpoint.env,
                ..ShellConfig::default()
            },
        );
        Self {
            id: SessionId::new(),
            tier: Tier::Isolate,
            fs,
            shell: Mutex::new(shell),
        }
    }
}

/// The wall-clock limit on one command: the caller's `timeout_ms` when it is
/// shorter than the session's wall timeout, and the session's wall timeout
/// otherwise. The second value says which one applies, for the error message.
pub(crate) fn effective_timeout(requested_ms: Option<u64>, wall_secs: u64) -> (Duration, String) {
    let wall = Duration::from_secs(wall_secs);
    match requested_ms.map(Duration::from_millis) {
        Some(requested) if requested < wall => (
            requested,
            format!("its {} ms timeout", requested.as_millis()),
        ),
        _ => (wall, format!("the session wall timeout of {wall_secs} s")),
    }
}

/// The session wall timeout, in seconds. Every isolate session uses the default,
/// because the server does not accept per-session limits yet (PLAN §4.1).
fn session_wall_secs() -> u64 {
    ResourceLimits::default().wall_timeout_secs.unwrap_or(300)
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

impl IsolateBackend {
    /// Runs one command for `exec` and `exec_streamed`. The timeout applies to
    /// both, and the observer, when there is one, sees the output as it is produced.
    async fn exec_with(
        &self,
        req: ExecRequest,
        observer: Option<OutputObserver>,
    ) -> Result<ExecResult, SandboxError> {
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
        let run = shell.run_argv_streamed(&req.argv, &stdin, observer);
        let (limit, reason) = effective_timeout(req.timeout_ms, session_wall_secs());
        // The shell runs no background tasks, and its loops are bounded by a
        // command budget, so dropping the command at its next await is safe.
        // Its partial writes stay in the filesystem, as they would after a kill.
        let out = tokio::time::timeout(limit, run).await;
        // A command cut short by the timeout never clears its observer. Clearing it
        // here releases the observer's channel, so a stream can end.
        shell.detach_observer();
        let out =
            out.map_err(|_| SandboxError::timeout(format!("command exceeded {reason}")))??;
        Ok(to_result(out, started))
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
                "a streaming request goes through exec_streamed",
            ));
        }
        self.exec_with(req, None).await
    }

    async fn exec_streamed(
        &self,
        req: ExecRequest,
        observer: OutputObserver,
    ) -> Result<ExecResult, SandboxError> {
        self.exec_with(req, Some(observer)).await
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

    async fn write_system_file(&self, path: &str, data: &[u8]) -> Result<(), SandboxError> {
        self.fs
            .write_system_file(path, data)
            .await
            .map_err(crate::filesystem::vfs_error)
    }

    async fn snapshot(&self) -> Result<ContentHash, SandboxError> {
        self.fs
            .checkpoint()
            .await
            .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SandboxError> {
        let root = self.snapshot().await?;
        // The shell lock keeps a command from changing the directory or the
        // environment while they are read.
        let shell = self.shell.lock().await;
        Ok(Checkpoint {
            root,
            cwd: shell.cwd().to_string(),
            env: shell.env().clone(),
        })
    }

    async fn destroy(self: Box<Self>) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// Starts a new session whose filesystem is `root`, a committed root returned
/// by `Session::snapshot`. The caller is responsible for who may use `root`.
pub async fn restore(root: ContentHash) -> Result<Box<dyn Session>, SandboxError> {
    restore_limited(root, &ResourceLimits::default()).await
}

/// Starts a session from a committed root, under `limits`.
pub async fn restore_limited(
    root: ContentHash,
    limits: &ResourceLimits,
) -> Result<Box<dyn Session>, SandboxError> {
    restore_checkpoint_limited(Checkpoint::root_only(root), limits).await
}

/// Starts a session from a checkpoint returned by `Session::checkpoint`. The
/// caller decides who may use it.
pub async fn restore_checkpoint(checkpoint: Checkpoint) -> Result<Box<dyn Session>, SandboxError> {
    restore_checkpoint_limited(checkpoint, &ResourceLimits::default()).await
}

/// Starts a session from a checkpoint, under `limits`.
pub async fn restore_checkpoint_limited(
    checkpoint: Checkpoint,
    limits: &ResourceLimits,
) -> Result<Box<dyn Session>, SandboxError> {
    Ok(Box::new(IsolateBackend::with_checkpoint(
        checkpoint, limits,
    )))
}

/// The filesystem quotas a session's limits set. A limit the caller leaves
/// unset takes its default. The VFS enforces these four. The others are not
/// enforced yet, and the server refuses them.
pub fn quotas_for(limits: &ResourceLimits) -> sandbox_vfs::Quotas {
    let defaults = sandbox_vfs::Quotas::default();
    sandbox_vfs::Quotas {
        total_bytes: limits.max_total_size.unwrap_or(defaults.total_bytes),
        inode_count: limits.max_inodes.unwrap_or(defaults.inode_count),
        max_file_size: limits.max_file_size.unwrap_or(defaults.max_file_size),
        max_path_depth: limits.max_path_depth.unwrap_or(defaults.max_path_depth),
    }
}

/// The committed root of a session's filesystem. Take one before a command, then
/// pass it to [`changed_since`] afterwards to list what the command changed.
pub async fn committed_root(session: &dyn Session) -> Result<ContentHash, SandboxError> {
    Ok(session.checkpoint().await?.root)
}

/// The paths that differ between `before`, an earlier [`committed_root`] of
/// `session`, and the session's filesystem now. Sorted by path. See
/// `sandbox_vfs::diff_trees`.
pub async fn changed_since(
    session: &dyn Session,
    before: ContentHash,
) -> Result<Vec<sandbox_vfs::TreeChange>, SandboxError> {
    let after = committed_root(session).await?;
    sandbox_vfs::diff_trees(&*shared_store(), before, after)
        .await
        .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_types::event::FileOp;
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
    async fn changed_since_lists_what_a_script_changed() {
        let backend = new_backend().await;
        backend.run_script("mkdir /work").await.unwrap();
        let before = committed_root(&backend).await.unwrap();
        backend.run_script("echo hi > /work/a.txt").await.unwrap();
        let changes = changed_since(&backend, before).await.unwrap();
        let listed: Vec<(&str, FileOp)> = changes.iter().map(|c| (c.path.as_str(), c.op)).collect();
        assert_eq!(listed, vec![("/work/a.txt", FileOp::Create)]);
        assert_eq!(changes[0].before, None);
        assert_eq!(changes[0].after, Some(ContentHash::new(b"hi\n")));
    }

    #[tokio::test]
    async fn a_command_that_changes_nothing_lists_no_files() {
        let backend = new_backend().await;
        backend
            .run_script("mkdir /w && echo x > /w/f")
            .await
            .unwrap();
        let before = committed_root(&backend).await.unwrap();
        backend.run_script("cat /w/f").await.unwrap();
        assert!(changed_since(&backend, before).await.unwrap().is_empty());
    }

    #[test]
    fn the_shorter_of_the_request_and_the_wall_timeout_applies() {
        assert_eq!(effective_timeout(None, 300).0, Duration::from_secs(300));
        assert_eq!(
            effective_timeout(Some(50), 300).0,
            Duration::from_millis(50)
        );
        assert_eq!(
            effective_timeout(Some(900_000), 300).0,
            Duration::from_secs(300)
        );
        assert!(effective_timeout(Some(900_000), 300)
            .1
            .contains("wall timeout"));
        assert!(effective_timeout(Some(50), 300).1.contains("50 ms"));
    }

    #[tokio::test]
    async fn exec_timeout_stops_a_long_command_with_a_timeout_error() {
        let _ = sandbox_runtime::install_in_process_script_runner();
        let backend = new_backend().await;
        let started = Instant::now();
        let mut req = argv(&["node", "-e", "let i=0; while(i<1000000000){i++}"]);
        req.timeout_ms = Some(50);
        let err = backend.exec(req).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn exec_streamed_reports_the_output_and_returns_it() {
        let backend = new_backend().await;
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let observer: OutputObserver = Arc::new(move |chunk| sink.lock().unwrap().push(chunk));
        let out = backend
            .exec_streamed(argv(&["echo", "hi"]), observer)
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, b"hi\n");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].fd, 1);
        assert_eq!(seen[0].data, b"hi\n");
    }

    #[tokio::test]
    async fn exec_refuses_a_streaming_request_so_it_is_not_silently_buffered() {
        let backend = new_backend().await;
        let mut req = argv(&["echo", "hi"]);
        req.stream = true;
        let err = backend.exec(req).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
    }

    #[tokio::test]
    async fn a_timed_out_streamed_exec_releases_its_observer() {
        let _ = sandbox_runtime::install_in_process_script_runner();
        // The stream ends when the observer's sender is dropped. A command cut short
        // by its timeout must not keep the observer alive inside the shell.
        let backend = new_backend().await;
        let mut req = argv(&["node", "-e", "let i=0; while(i<1000000000){i++}"]);
        req.timeout_ms = Some(50);
        let observer: OutputObserver = Arc::new(|_| {});
        let held = observer.clone();
        let err = backend.exec_streamed(req, observer).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the shell still holds the observer"
        );
    }

    #[tokio::test]
    async fn exec_within_its_timeout_returns_the_output() {
        let backend = new_backend().await;
        let mut req = argv(&["echo", "quick"]);
        req.timeout_ms = Some(5_000);
        let out = backend.exec(req).await.unwrap();
        assert_eq!(out.stdout, b"quick\n".to_vec());
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
    async fn streaming_exec_is_refused_not_ignored() {
        let backend = new_backend().await;
        let streamed = ExecRequest {
            stream: true,
            ..argv(&["true"])
        };
        assert_eq!(
            backend.exec(streamed).await.err().unwrap().code,
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
    async fn checkpoint_restores_files_and_shell_state() {
        let backend = new_backend().await;
        write_text(backend.fs(), "/keep.txt", "kept").await;
        backend
            .fs()
            .mkdir(std::path::Path::new("/work"), 0o755)
            .await
            .unwrap();
        backend
            .run_script("export GREETING=hello; cd /work")
            .await
            .unwrap();
        let checkpoint = backend.checkpoint().await.unwrap();
        // Writes after the checkpoint belong to the session that is gone.
        write_text(backend.fs(), "/keep.txt", "later").await;

        let woken = restore_checkpoint(checkpoint).await.unwrap();
        assert_eq!(read_text(woken.fs(), "/keep.txt").await, "kept");
        let out = woken.run_script("echo $GREETING; pwd").await.unwrap();
        assert_eq!(out.stdout, b"hello\n/work\n");
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
