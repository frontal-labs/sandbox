//! Worker processes: one child process per session.
//!
//! The child serves `sandbox.v1.worker.WorkerService` (gRPC) on a private Unix
//! socket. The socket lives in a directory the host creates with mode `0700`,
//! so only the same user can reach it. Guest output never shares a channel
//! with the protocol: scripts write to their own captured buffers, not to the
//! worker's stdout.
//!
//! The host starts each worker with an [`Isolation`]: namespaces and seccomp are
//! applied by the child before its runtime starts, and the host moves the child
//! into a cgroup with resource limits. See [`crate::isolation`] and PLAN.md §4.13.

#[cfg(target_os = "linux")]
use crate::isolation::CgroupGuard;
use crate::isolation::Isolation;
use sandbox_proto::status::{from_status, to_status};
use sandbox_proto::v1::types as pb;
use sandbox_proto::v1::worker::worker_service_client::WorkerServiceClient;
use sandbox_proto::v1::worker::worker_service_server::{WorkerService, WorkerServiceServer};
use sandbox_proto::v1::worker::{HealthResponse, RunScriptRequest};
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::{Request, Response, Status};

/// Largest request or response message accepted on the worker channel (16 MiB).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// How long the host waits for a freshly spawned worker to answer `Health`.
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// Deadline for one script or command. Enforced by the host and sent to the worker.
const RPC_DEADLINE: Duration = Duration::from_secs(60);

// --- Child side -------------------------------------------------------------

struct WorkerImpl {
    session: Arc<dyn Session>,
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
}

#[tonic::async_trait]
impl WorkerService for WorkerImpl {
    async fn run_script(
        &self,
        request: Request<RunScriptRequest>,
    ) -> Result<Response<pb::ExecResult>, Status> {
        let script = request.into_inner().script;
        let result = self
            .session
            .run_script(&script)
            .await
            .map_err(|e| to_status(&e))?;
        let out = pb::ExecResult::from(result);
        check_encoded_size(&out, "result").map_err(|e| to_status(&e))?;
        Ok(Response::new(out))
    }

    async fn exec(
        &self,
        request: Request<pb::ExecRequest>,
    ) -> Result<Response<pb::ExecResult>, Status> {
        let req = ExecRequest::try_from(request.into_inner()).map_err(|e| to_status(&e))?;
        let result = self.session.exec(req).await.map_err(|e| to_status(&e))?;
        let out = pb::ExecResult::from(result);
        check_encoded_size(&out, "result").map_err(|e| to_status(&e))?;
        Ok(Response::new(out))
    }

    async fn health(&self, _: Request<()>) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            ready: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn shutdown(&self, _: Request<()>) -> Result<Response<()>, Status> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(Response::new(()))
    }
}

/// Serves one fresh session on `socket` until a `Shutdown` RPC arrives.
pub async fn serve(socket: &Path) -> Result<(), SandboxError> {
    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket)
        .map_err(|e| SandboxError::unavailable(format!("binding worker socket: {e}")))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| SandboxError::internal(format!("restricting worker socket: {e}")))?;

    let session: Arc<dyn Session> = Arc::new(IsolateBackend::new(SessionConfig::default()).await?);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let service = WorkerServiceServer::new(WorkerImpl {
        session,
        shutdown: Mutex::new(Some(shutdown_tx)),
    })
    .max_decoding_message_size(MAX_MESSAGE_BYTES)
    .max_encoding_message_size(MAX_MESSAGE_BYTES);

    Server::builder()
        .add_service(service)
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::UnixListenerStream::new(listener),
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
        .map_err(|e| SandboxError::unavailable(format!("worker server failed: {e}")))?;
    let _ = std::fs::remove_file(socket);
    Ok(())
}

// --- Host side --------------------------------------------------------------

/// Host handle for one worker process. Dropping it kills the child and removes
/// its socket directory.
pub struct Worker {
    child: Child,
    client: WorkerServiceClient<Channel>,
    dir: PathBuf,
    /// The worker's cgroup, removed on drop once the child has exited.
    #[cfg(target_os = "linux")]
    cgroup: Option<CgroupGuard>,
    isolation: Isolation,
}

impl Worker {
    /// Starts `program worker --socket <path>` with the default isolation, and waits
    /// until it answers `Health`. On Linux that is the strict isolation.
    pub async fn spawn(program: impl AsRef<std::ffi::OsStr>) -> Result<Self, SandboxError> {
        Self::spawn_with(program, Isolation::default()).await
    }

    /// Starts a worker with `isolation`. Fails, and starts nothing, if a layer the
    /// isolation asks for cannot be applied. A worker is never served without the
    /// layers it was asked for.
    pub async fn spawn_with(
        program: impl AsRef<std::ffi::OsStr>,
        isolation: Isolation,
    ) -> Result<Self, SandboxError> {
        let dir = private_dir()?;
        let socket = dir.join("worker.sock");
        // The cgroup is created before the child starts, so a failure to set its
        // limits starts nothing.
        #[cfg(target_os = "linux")]
        let cgroup = match isolation.cgroup {
            Some(limits) => {
                let name = format!("sbx-{}", &ulid::Ulid::new().to_string()[10..]);
                match CgroupGuard::create(&name, limits) {
                    Ok(guard) => Some(guard),
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&dir);
                        return Err(e);
                    }
                }
            }
            None => None,
        };
        #[cfg(not(target_os = "linux"))]
        if isolation.cgroup.is_some() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(SandboxError::new(
                ErrorCode::Unsupported,
                "cgroup limits need Linux",
            ));
        }
        let mut command = Command::new(program);
        command
            .arg("worker")
            .arg("--socket")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        // The child applies these itself, before its runtime starts (see `enter`).
        if isolation.namespaces {
            command.arg("--namespaces");
        }
        if isolation.seccomp {
            command.arg("--seccomp");
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(SandboxError::unavailable(format!(
                    "failed to start worker: {e}"
                )));
            }
        };
        #[cfg(target_os = "linux")]
        if let (Some(guard), Some(pid)) = (&cgroup, child.id()) {
            if let Err(e) = guard.add(pid) {
                let _ = child.kill().await;
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        }
        let channel = match connect_when_ready(&mut child, &socket).await {
            Ok(channel) => channel,
            Err(e) => {
                let _ = child.kill().await;
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        let client = WorkerServiceClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
        Ok(Self {
            child,
            client,
            dir,
            #[cfg(target_os = "linux")]
            cgroup,
            isolation,
        })
    }

    /// The isolation this worker was started with.
    pub fn isolation(&self) -> &Isolation {
        &self.isolation
    }

    /// Runs a script in the worker. A script error comes back as its own
    /// `ErrorCode`. A dead or unreachable worker returns `Unavailable`.
    pub async fn run_script(&mut self, script: &str) -> Result<ExecResult, SandboxError> {
        let message = RunScriptRequest {
            script: script.to_string(),
        };
        // Checked before sending, so the caller gets `ResourceExhausted` instead of a reset stream.
        check_encoded_size(&message, "script")?;
        let mut request = Request::new(message);
        request.set_timeout(RPC_DEADLINE);
        let response = self.client.run_script(request).await.map_err(map_status)?;
        ExecResult::try_from(response.into_inner())
    }

    /// Runs one command in the worker's session.
    pub async fn exec(&mut self, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        let message = pb::ExecRequest::from(req);
        check_encoded_size(&message, "exec request")?;
        let mut request = Request::new(message);
        request.set_timeout(RPC_DEADLINE);
        let response = self.client.exec(request).await.map_err(map_status)?;
        ExecResult::try_from(response.into_inner())
    }

    /// The child's process id, if it is still known.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// The private directory holding the worker's socket. Removed on drop.
    pub fn socket_dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the child is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Asks the worker to stop, then waits for the child to exit.
    pub async fn shutdown(mut self) {
        let mut request = Request::new(());
        request.set_timeout(Duration::from_secs(5));
        let _ = self.client.shutdown(request).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_dir_all(&self.dir);
        // Removed last. The kernel refuses to remove a cgroup while a process is in
        // it, so the guard's own drop succeeds only once the child has exited.
        #[cfg(target_os = "linux")]
        drop(self.cgroup.take());
    }
}

/// Refuses a message whose encoded size is over `MAX_MESSAGE_BYTES`. Used on both
/// sides, so an oversize request or result gets `ResourceExhausted` rather than a
/// transport error.
fn check_encoded_size<M: prost::Message>(message: &M, what: &str) -> Result<(), SandboxError> {
    let len = message.encoded_len();
    if len > MAX_MESSAGE_BYTES {
        return Err(SandboxError::new(
            ErrorCode::ResourceExhausted,
            format!("{what} of {len} bytes exceeds the {MAX_MESSAGE_BYTES} byte limit"),
        ));
    }
    Ok(())
}

fn map_status(status: Status) -> SandboxError {
    if status.code() == tonic::Code::Unavailable {
        return SandboxError::unavailable(format!("worker is not reachable: {}", status.message()));
    }
    from_status(&status)
}

/// Creates a fresh directory with mode `0700`. Its name is short, so the socket
/// path stays under the platform's Unix socket path limit.
fn private_dir() -> Result<PathBuf, SandboxError> {
    // A ULID starts with 10 timestamp characters, so only the last 16 are random.
    // Keeping just a short prefix made two workers started in the same millisecond
    // collide.
    let name = format!("sbx-{}", &ulid::Ulid::new().to_string()[10..]);
    let dir = std::env::temp_dir().join(name);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| SandboxError::internal(format!("creating worker directory: {e}")))?;
    Ok(dir)
}

/// Connects to the worker's socket, retrying until it is up, the child exits, or
/// `READY_TIMEOUT` passes. Ends with a successful `Health` call.
async fn connect_when_ready(child: &mut Child, socket: &Path) -> Result<Channel, SandboxError> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(SandboxError::unavailable(format!(
                "worker exited before ready: {status}"
            )));
        }
        match connect(socket).await {
            Ok(channel) => {
                let mut client = WorkerServiceClient::new(channel.clone());
                let mut request = Request::new(());
                request.set_timeout(Duration::from_secs(2));
                if client.health(request).await.is_ok() {
                    return Ok(channel);
                }
            }
            Err(_) if Instant::now() < deadline => {}
            Err(e) => return Err(e),
        }
        if Instant::now() >= deadline {
            return Err(SandboxError::new(
                ErrorCode::Timeout,
                "worker did not become ready",
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn connect(socket: &Path) -> Result<Channel, SandboxError> {
    let socket = socket.to_path_buf();
    Endpoint::try_from("http://sandbox-worker.local")
        .map_err(|e| SandboxError::internal(format!("bad endpoint: {e}")))?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move {
                let stream = UnixStream::connect(socket).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .map_err(|e| SandboxError::unavailable(format!("connecting to worker: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_child_that_exits_at_once_is_unavailable_not_a_panic() {
        // `true` ignores its arguments and exits, so the worker never becomes ready.
        // It is started unisolated, so this checks readiness, not the isolation setup.
        let err = match Worker::spawn_with("true", Isolation::none()).await {
            Ok(_) => panic!("a non-worker process must not become ready"),
            Err(err) => err,
        };
        assert_eq!(err.code, ErrorCode::Unavailable, "{}", err.message);
    }

    #[test]
    fn oversize_scripts_are_refused_before_sending() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // A channel that never connects: the size check must fail first.
            let endpoint = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
            let mut worker = Worker {
                child: Command::new("sleep").arg("30").spawn().unwrap(),
                client: WorkerServiceClient::new(endpoint),
                dir: std::env::temp_dir(),
                #[cfg(target_os = "linux")]
                cgroup: None,
                isolation: Isolation::none(),
            };
            let big = "x".repeat(MAX_MESSAGE_BYTES + 1);
            let err = worker.run_script(&big).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::ResourceExhausted);
            // Exec is checked the same way: stdin alone can push the message over the limit.
            let req = ExecRequest {
                argv: vec!["cat".into()],
                env: Default::default(),
                cwd: None,
                stdin: Some(vec![0u8; MAX_MESSAGE_BYTES]),
                timeout_ms: None,
                stream: false,
            };
            let err = worker.exec(req).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::ResourceExhausted);
            let _ = worker.child.kill().await;
            // Do not run Drop: it would remove the shared temp directory.
            std::mem::forget(worker);
        });
    }
}
