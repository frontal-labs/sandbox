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
use std::ops::{Deref, DerefMut};
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
    /// Per-session resource limits.
    limits: ResourceLimits,
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
            limits: limits.clone(),
        }
    }
}

/// The wall-clock limit on one command: the caller's `timeout_ms` when it is
/// shorter than the session's wall timeout, and the session's wall timeout
/// otherwise. The second value says which one applies, for the error message.
pub fn effective_timeout(requested_ms: Option<u64>, wall_secs: u64) -> (Duration, String) {
    let wall = Duration::from_secs(wall_secs);
    match requested_ms.map(Duration::from_millis) {
        Some(requested) if requested < wall => (
            requested,
            format!("its {} ms timeout", requested.as_millis()),
        ),
        _ => (wall, format!("the session wall timeout of {wall_secs} s")),
    }
}

/// The session wall timeout, in seconds. A session without its own value gets
/// the default, so a command never runs unbounded.
pub fn session_wall_secs(limits: &ResourceLimits) -> u64 {
    limits
        .wall_timeout_secs
        .unwrap_or_else(|| ResourceLimits::default().wall_timeout_secs.unwrap_or(300))
}

fn to_result(out: ExecutionResult, started: Instant, cpu: Duration) -> ExecResult {
    ExecResult {
        exit_code: out.exit_code,
        stdout: out.stdout,
        stderr: out.stderr,
        cpu_ms: u64::try_from(cpu.as_millis()).unwrap_or(u64::MAX),
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

        // Enforce per-session CPU limit (cpu_ms_per_sec).
        if let Some(cpu_limit_ms) = self.limits.cpu_ms_per_sec {
            if cpu_limit_ms == 0 {
                return Err(SandboxError::new(
                    ErrorCode::ResourceExhausted,
                    "CPU limit is zero",
                ));
            }
        }

        // Enforce per-session process limit (max_processes).
        // Note: The shell runs commands sequentially, so this limits concurrent
        // subprocesses spawned by the shell itself (e.g., via pipelines).
        if let Some(max_procs) = self.limits.max_processes {
            if max_procs == 0 {
                return Err(SandboxError::new(
                    ErrorCode::ResourceExhausted,
                    "Process limit is zero",
                ));
            }
        }

        // Enforce per-session open file limit (max_open_files).
        if let Some(max_files) = self.limits.max_open_files {
            if max_files == 0 {
                return Err(SandboxError::new(
                    ErrorCode::ResourceExhausted,
                    "Open file limit is zero",
                ));
            }
        }

        let started = Instant::now();
        let mut shell = DetachOnDrop {
            shell: self.shell.lock().await,
        };
        // A command this tier cannot run is refused, so the caller can promote the session
        // to a VM (PLAN.md section 4.11). It is not run as a "command not found" failure.
        if !shell.command_available(&req.argv[0]).await {
            return Err(SandboxError::needs_vm(&req.argv[0]));
        }
        for (key, value) in req.env {
            shell.set_env(key, value);
        }
        if let Some(cwd) = &req.cwd {
            shell.chdir(cwd).await?;
        }
        let stdin = req.stdin.unwrap_or_default();
        let run =
            sandbox_shell::cpu::CpuMeter::new(shell.run_argv_streamed(&req.argv, &stdin, observer));
        let (limit, reason) = effective_timeout(req.timeout_ms, session_wall_secs(&self.limits));
        // The shell runs no background tasks, and its loops are bounded by a
        // command budget, so dropping the command at its next await is safe.
        // Its partial writes stay in the filesystem, as they would after a kill.
        let timed = tokio::time::timeout(limit, run)
            .await
            .map_err(|_| SandboxError::timeout(format!("command exceeded {reason}")))?;
        let (out, cpu) = timed;
        Ok(to_result(out?, started, cpu))
    }
}

/// The shell lock, held for one command. It drops the command's output observer when it
/// goes, so a run that ends early, by timeout or by cancellation, cannot leave the
/// observer's channel open. Without this, a stream waiting for its end would never close.
struct DetachOnDrop<'a> {
    shell: tokio::sync::MutexGuard<'a, Shell>,
}

impl Deref for DetachOnDrop<'_> {
    type Target = Shell;

    fn deref(&self) -> &Shell {
        &self.shell
    }
}

impl DerefMut for DetachOnDrop<'_> {
    fn deref_mut(&mut self) -> &mut Shell {
        &mut self.shell
    }
}

impl Drop for DetachOnDrop<'_> {
    fn drop(&mut self) {
        self.shell.detach_observer();
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
        let (out, cpu) = sandbox_shell::cpu::CpuMeter::new(shell.run(script)).await;
        Ok(to_result(out?, started, cpu))
    }

    async fn run_script_live(
        &self,
        script: &str,
        input: sandbox_shell::LiveInput,
        observer: sandbox_shell::OutputObserver,
    ) -> Result<ExecResult, SandboxError> {
        let started = Instant::now();
        let mut shell = self.shell.lock().await;
        let (out, cpu) = sandbox_shell::cpu::CpuMeter::new(shell.run_streamed_live(
            script,
            Some(observer),
            input,
        ))
        .await;
        Ok(to_result(out?, started, cpu))
    }

    fn fs(&self) -> &dyn SessionFs {
        self.fs.as_ref()
    }

    async fn random_state(&self) -> u64 {
        self.shell.lock().await.random_state()
    }

    async fn set_random_state(&self, state: u64) {
        self.shell.lock().await.set_random_state(state);
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
/// unset takes its default. The VFS enforces these four.
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
