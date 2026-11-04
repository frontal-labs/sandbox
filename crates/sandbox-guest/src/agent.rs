use sandbox_proto::v1::guest::{
    guest_service_server::GuestService, guest_service_server::GuestServiceServer, DirEntry,
    ListDirRequest, ListDirResponse, MkDirRequest, PingResponse, ReadFileRequest, ReadFileResponse,
    RemoveRequest, RenameRequest, StatRequest, StatResponse, SyncRequest, SyncResponse,
    WriteFileRequest,
};
use sandbox_types::{ContentHash, SandboxError};
use sandbox_vfs::{SessionFs, Vfs};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, UnixListener};
use tokio_stream::wrappers::{TcpListenerStream, UnixListenerStream};
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;

/// Where the agent accepts connections. A Firecracker guest uses a Unix socket. The
/// emulated test guest (`docker/emulated-vm/run.sh`) uses TCP over a forwarded port.
enum Listener {
    Unix(UnixListener),
    Tcp(TcpListener),
}

/// A chunk the agent returns when the request names no size.
const DEFAULT_CHUNK_BYTES: usize = 1024 * 1024;
/// The largest chunk the agent moves in one message, under the transport's 4 MiB limit.
const MAX_CHUNK_BYTES: usize = 2 * 1024 * 1024;

pub struct GuestAgent {
    vfs: Arc<Vfs>,
    listener: Listener,
}

impl GuestAgent {
    pub async fn new(socket_path: &str, vfs: Arc<Vfs>) -> Result<Self, SandboxError> {
        // Remove existing socket if it exists
        let _ = tokio::fs::remove_file(socket_path).await;

        let listener = UnixListener::bind(socket_path).map_err(|e| {
            SandboxError::internal(format!("Failed to bind to {}: {}", socket_path, e))
        })?;

        Ok(Self {
            vfs,
            listener: Listener::Unix(listener),
        })
    }

    /// Binds a TCP address, such as `0.0.0.0:7000`.
    pub async fn new_tcp(addr: &str, vfs: Arc<Vfs>) -> Result<Self, SandboxError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to bind to {addr}: {e}")))?;
        Ok(Self {
            vfs,
            listener: Listener::Tcp(listener),
        })
    }

    pub async fn run(self) -> Result<(), SandboxError> {
        info!("Guest agent listening");

        let service = GuestServiceServer::new(GuestServiceImpl { vfs: self.vfs });
        let served = match self.listener {
            Listener::Unix(listener) => {
                Server::builder()
                    .add_service(service)
                    .serve_with_incoming(UnixListenerStream::new(listener))
                    .await
            }
            Listener::Tcp(listener) => {
                Server::builder()
                    .add_service(service)
                    .serve_with_incoming(TcpListenerStream::new(listener))
                    .await
            }
        };
        served.map_err(|e| SandboxError::internal(format!("Guest server error: {e}")))
    }
}

#[derive(Clone)]
struct GuestServiceImpl {
    vfs: Arc<Vfs>,
}

#[tonic::async_trait]
impl GuestService for GuestServiceImpl {
    async fn ping(&self, _request: Request<()>) -> Result<Response<PingResponse>, Status> {
        Ok(Response::new(PingResponse {
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn exec(
        &self,
        request: Request<sandbox_proto::v1::types::ExecRequest>,
    ) -> Result<Response<sandbox_proto::v1::types::ExecResult>, Status> {
        let req = request.into_inner();
        let argv = req.argv;
        let env: Vec<(String, String)> = req.env.into_iter().collect();
        let cwd = req.cwd;
        let stdin = req.stdin;

        let mut shell =
            sandbox_shell::Shell::new(self.vfs.clone(), sandbox_shell::ShellConfig::default());
        for (k, v) in env {
            shell.set_env(k, v);
        }
        if let Some(cwd) = cwd {
            shell
                .chdir(&cwd)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
        }

        let started = std::time::Instant::now();
        let (out, cpu) = sandbox_shell::cpu::CpuMeter::new(shell.run_argv_streamed(
            &argv,
            &stdin.unwrap_or_default(),
            None,
        ))
        .await;
        let out = out.map_err(|e| Status::internal(e.to_string()))?;

        let exec_result = sandbox_types::ExecResult {
            exit_code: out.exit_code,
            stdout: out.stdout,
            stderr: out.stderr,
            cpu_ms: u64::try_from(cpu.as_millis()).unwrap_or(u64::MAX),
            wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };

        Ok(Response::new(exec_result.into()))
    }

    async fn read_file(
        &self,
        request: Request<ReadFileRequest>,
    ) -> Result<Response<ReadFileResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);
        let max = match req.max_bytes {
            0 => DEFAULT_CHUNK_BYTES,
            n => (n as usize).min(MAX_CHUNK_BYTES),
        };

        let handle = self
            .vfs
            .open(&path, sandbox_vfs::OpenFlags::default())
            .await
            .map_err(|e| Status::not_found(format!("File not found: {}", e)))?;
        let chunk = self
            .vfs
            .read(&handle, req.offset, max)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        self.vfs.close(&handle);
        Ok(Response::new(ReadFileResponse {
            eof: chunk.len() < max,
            data: chunk.to_vec(),
        }))
    }

    async fn write_file(&self, request: Request<WriteFileRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);
        if req.data.len() > MAX_CHUNK_BYTES {
            return Err(Status::invalid_argument(
                "chunk is larger than the agent accepts",
            ));
        }

        // A write creates the file if needed and changes only the bytes it names. The
        // caller asks for a truncation when it wants the file emptied first.
        let flags = sandbox_vfs::OpenFlags {
            read: false,
            write: true,
            create: true,
            truncate: req.truncate,
            ..Default::default()
        };
        let handle = self
            .vfs
            .open(&path, flags)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let written = self
            .vfs
            .write(&handle, req.offset, &req.data)
            .await
            .map_err(|e| Status::internal(e.to_string()));
        self.vfs.close(&handle);
        written?;
        Ok(Response::new(()))
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<Response<StatResponse>, Status> {
        let path = PathBuf::from(request.into_inner().path);
        let stat = self
            .vfs
            .stat(&path)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;
        Ok(Response::new(StatResponse {
            is_dir: stat.is_dir,
            is_file: stat.is_file,
            size: stat.size,
        }))
    }

    async fn remove(&self, request: Request<RemoveRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        self.vfs
            .rm(&PathBuf::from(req.path), req.recursive)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(()))
    }

    async fn rename(&self, request: Request<RenameRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        self.vfs
            .rename(&PathBuf::from(req.from), &PathBuf::from(req.to))
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(()))
    }

    async fn mk_dir(&self, request: Request<MkDirRequest>) -> Result<Response<()>, Status> {
        let path = PathBuf::from(request.into_inner().path);
        match self.vfs.mkdir(&path, 0o755).await {
            Ok(()) => Ok(Response::new(())),
            // Promotion may repeat a directory it already made. That is not an error.
            Err(_) if self.vfs.stat(&path).await.is_ok_and(|s| s.is_dir) => Ok(Response::new(())),
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn list_dir(
        &self,
        request: Request<ListDirRequest>,
    ) -> Result<Response<ListDirResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);

        let entries = self
            .vfs
            .readdir(&path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let entries = entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                is_dir: e.is_dir,
                size: e.size,
            })
            .collect();

        Ok(Response::new(ListDirResponse { entries }))
    }

    async fn sync(&self, request: Request<SyncRequest>) -> Result<Response<SyncResponse>, Status> {
        let req = request.into_inner();
        // The root the guest holds now. The VFS commits its tree to the guest's own store.
        let root = self
            .vfs
            .checkpoint()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        if let Some(expected) = req.expected_root {
            let expected = ContentHash::try_from(expected)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            if expected != root {
                // The caller asked about a different tree. Reporting the root the guest
                // holds lets the caller decide what to do; nothing here is changed.
                return Err(Status::failed_precondition(format!(
                    "the guest holds root {root}, not {expected}"
                )));
            }
        }
        Ok(Response::new(SyncResponse {
            root: Some(root.into()),
        }))
    }
}

pub async fn start_guest_agent(socket_path: &str, vfs: Arc<Vfs>) -> Result<(), SandboxError> {
    let agent = GuestAgent::new(socket_path, vfs).await?;
    agent.run().await
}
