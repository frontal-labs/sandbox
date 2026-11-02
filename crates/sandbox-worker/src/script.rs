//! Contained `node` scripts: one child process per script.
//!
//! The host ([`ScriptProcess`]) starts `program script-run` with namespaces and
//! seccomp applied by the child, and moves it into a cgroup whose memory limit is
//! the script's heap limit plus a fixed overhead for V8 itself. If the script
//! exhausts that memory, the kernel kills only the child, and the host reports it as
//! a script failure. A fatal V8 out-of-memory in the child cannot take down the host.
//!
//! The child ([`run_script_child`]) reads one JSON request from stdin, runs the
//! script with `sandbox-runtime`, and writes frames to stdout. A frame is one tag
//! byte, a big-endian `u32` length, then the payload:
//!
//! - `O`: a piece of stdout, in order.
//! - `E`: the stderr text, once, at the end.
//! - `X`: the exit code as a big-endian `i32`, once, at the end.
//!
//! A child that ends without `X` did not finish, and the host reports why.

use crate::isolation::{enter, Isolation};
use async_trait::async_trait;
use sandbox_runtime::{
    execute_script_streamed, ScriptLimits, ScriptOutcome, ScriptRunner, StreamObserver,
};
#[cfg(not(target_os = "linux"))]
use sandbox_types::ErrorCode;
use sandbox_types::SandboxError;
use sandbox_vfs::Vfs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
#[cfg(target_os = "linux")]
use tokio::io::AsyncReadExt;

#[cfg(target_os = "linux")]
/// Memory the child may use beyond the script's heap limit: V8 itself, the runtime,
/// and the thread stacks.
const SCRIPT_OVERHEAD_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(target_os = "linux")]
/// How long past the script's own timeout the host waits before killing the child.
const HOST_GRACE: Duration = Duration::from_secs(2);
/// The largest frame the host accepts, so a broken child cannot make it allocate freely.
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const TAG_STDOUT: u8 = b'O';
const TAG_STDERR: u8 = b'E';
const TAG_EXIT: u8 = b'X';

/// The subcommand the child is started with. The host binary must provide it.
pub const SCRIPT_RUN_SUBCOMMAND: &str = "script-run";

/// The JSON request the host writes to the child's stdin.
#[derive(serde::Serialize, serde::Deserialize)]
struct ScriptRequest {
    code: String,
    timeout_ms: u64,
    max_output_bytes: usize,
    max_heap_bytes: usize,
}

/// Runs each `node` script in a fresh, isolated child process.
#[derive(Debug, Clone)]
pub struct ScriptProcess {
    program: PathBuf,
}

impl ScriptProcess {
    /// Uses `program`, a binary that provides the `script-run` subcommand.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The binary the children are started from.
    pub fn program(&self) -> &Path {
        &self.program
    }
}

#[async_trait]
impl ScriptRunner for ScriptProcess {
    async fn run(
        &self,
        code: &str,
        limits: ScriptLimits,
        observer: Option<StreamObserver>,
        vfs: Option<Arc<Vfs>>,
    ) -> Result<ScriptOutcome, SandboxError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (code, limits, observer, vfs, &self.program);
            Err(SandboxError::new(
                ErrorCode::Unsupported,
                "contained scripts need Linux namespaces, seccomp, and cgroups",
            ))
        }
        #[cfg(target_os = "linux")]
        {
            run_contained(&self.program, code, limits, observer, vfs).await
        }
    }
}

#[cfg(target_os = "linux")]
async fn run_contained(
    program: &Path,
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> Result<ScriptOutcome, SandboxError> {
    use crate::isolation::{CgroupGuard, CgroupLimits};
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    // The cgroup is created before the child starts, so a missing cgroup starts nothing.
    let cgroup_limits = CgroupLimits {
        memory_bytes: limits.max_heap_bytes as u64 + SCRIPT_OVERHEAD_BYTES,
        ..CgroupLimits::default()
    };
    let name = format!("sbx-script-{}", &ulid::Ulid::new().to_string()[10..]);
    let cgroup = CgroupGuard::create(&name, cgroup_limits)?;

    let mut command = Command::new(program);
    command
        .arg(SCRIPT_RUN_SUBCOMMAND)
        .arg("--namespaces")
        .arg("--seccomp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| SandboxError::unavailable(format!("failed to start script process: {e}")))?;
    if let Some(pid) = child.id() {
        if let Err(e) = cgroup.add(pid) {
            let _ = child.kill().await;
            return Err(e);
        }
    }

    let request = ScriptRequest {
        code: code.to_string(),
        timeout_ms: limits.timeout.as_millis() as u64,
        max_output_bytes: limits.max_output_bytes,
        max_heap_bytes: limits.max_heap_bytes,
    };
    let body = serde_json::to_vec(&request)
        .map_err(|e| SandboxError::internal(format!("encoding script request: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A child that died early fails this write. Its stdout then ends, and the
        // read below reports why.
        let _ = stdin.write_all(&body).await;
        let _ = stdin.shutdown().await;
    }

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut collected = Collected::default();
    let deadline = limits.timeout + HOST_GRACE;
    let read = tokio::time::timeout(
        deadline,
        read_frames(&mut stdout, &limits, observer, &mut collected),
    )
    .await;
    let timed_out = read.is_err();
    let protocol_error = matches!(read, Ok(Err(_)));
    if timed_out || protocol_error {
        let _ = child.kill().await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let oom_kills = cgroup.oom_kills();
    drop(cgroup);

    if let Some(exit_code) = collected.exit_code {
        return Ok(ScriptOutcome {
            exit_code,
            stdout: collected.stdout,
            stderr: collected.stderr,
        });
    }
    let reason = if timed_out {
        format!("script timed out after {:?}", limits.timeout)
    } else if oom_kills > 0 {
        format!(
            "script exceeded the memory limit of {} bytes",
            limits.max_heap_bytes
        )
    } else if protocol_error {
        "script process sent an invalid result".to_string()
    } else {
        "script process ended without a result".to_string()
    };
    let mut stderr = collected.stderr;
    stderr.extend_from_slice(reason.as_bytes());
    stderr.push(b'\n');
    Ok(ScriptOutcome {
        exit_code: 1,
        stdout: collected.stdout,
        stderr,
    })
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Collected {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: Option<i32>,
}

#[cfg(target_os = "linux")]
/// Reads frames until the child closes stdout, keeping the stdout capped at the
/// script's output limit. A protocol error ends the read with an error.
async fn read_frames(
    stdout: &mut tokio::process::ChildStdout,
    limits: &ScriptLimits,
    observer: Option<StreamObserver>,
    collected: &mut Collected,
) -> Result<(), SandboxError> {
    loop {
        let mut tag = [0u8; 1];
        match stdout.read_exact(&mut tag).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(SandboxError::internal(format!("reading script frame: {e}"))),
        }
        let mut len = [0u8; 4];
        stdout
            .read_exact(&mut len)
            .await
            .map_err(|e| SandboxError::internal(format!("reading script frame: {e}")))?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(SandboxError::internal("script frame is too large"));
        }
        let mut payload = vec![0u8; len];
        stdout
            .read_exact(&mut payload)
            .await
            .map_err(|e| SandboxError::internal(format!("reading script frame: {e}")))?;
        match tag[0] {
            TAG_STDOUT => {
                // The child already caps its output. This keeps the host's own bound
                // even against a child that does not.
                let room = limits
                    .max_output_bytes
                    .saturating_sub(collected.stdout.len());
                let kept = &payload[..payload.len().min(room)];
                collected.stdout.extend_from_slice(kept);
                if let (Some(observer), false) = (&observer, kept.is_empty()) {
                    observer(kept);
                }
            }
            TAG_STDERR => collected.stderr.extend_from_slice(&payload),
            TAG_EXIT => {
                let bytes: [u8; 4] = payload
                    .as_slice()
                    .try_into()
                    .map_err(|_| SandboxError::internal("script exit frame is malformed"))?;
                collected.exit_code = Some(i32::from_be_bytes(bytes));
                return Ok(());
            }
            other => {
                return Err(SandboxError::internal(format!(
                    "unknown script frame tag {other}"
                )))
            }
        }
    }
}

/// The child side: applies `isolation`, runs one script from stdin, and writes frames
/// to stdout. Call it before any thread or tokio runtime exists, because namespaces
/// and seccomp apply to the calling thread and to threads created after.
pub fn run_script_child(isolation: &Isolation) -> Result<(), SandboxError> {
    enter(isolation)?;

    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| SandboxError::internal(format!("reading script request: {e}")))?;
    let request: ScriptRequest = serde_json::from_slice(&input)
        .map_err(|e| SandboxError::internal(format!("decoding script request: {e}")))?;
    let limits = ScriptLimits {
        timeout: Duration::from_millis(request.timeout_ms),
        max_output_bytes: request.max_output_bytes,
        max_heap_bytes: request.max_heap_bytes,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SandboxError::internal(format!("starting script runtime: {e}")))?;
    let observer: StreamObserver = Arc::new(|piece: &[u8]| {
        // Split into frames the host accepts, so one large print is not refused.
        for chunk in piece.chunks(MAX_FRAME_BYTES) {
            let _ = write_frame(TAG_STDOUT, chunk);
        }
    });
    let (exit_code, stderr) = match runtime.block_on(execute_script_streamed(
        &request.code,
        limits,
        Some(observer),
        None,
    )) {
        Ok(outcome) => (outcome.exit_code, outcome.stderr),
        Err(e) => (1, e.message.into_bytes()),
    };
    write_frame(TAG_STDERR, &stderr)
        .and_then(|_| write_frame(TAG_EXIT, &exit_code.to_be_bytes()))
        .map_err(|e| SandboxError::internal(format!("writing script result: {e}")))
}

/// Writes one frame to stdout, flushing it so the host sees it at once.
fn write_frame(tag: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(&[tag])?;
    out.write_all(&(payload.len() as u32).to_be_bytes())?;
    out.write_all(payload)?;
    out.flush()
}
