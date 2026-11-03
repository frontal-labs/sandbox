use sandbox_proto::v1::guest::{
    guest_service_server::GuestService, guest_service_server::GuestServiceServer, DirEntry,
    ListDirRequest, ListDirResponse, PingResponse, ReadFileRequest, ReadFileResponse, SyncRequest,
    SyncResponse, WriteFileRequest,
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

        let flags = sandbox_vfs::OpenFlags::default();
        let handle = self
            .vfs
            .open(&path, flags)
            .await
            .map_err(|e| Status::not_found(format!("File not found: {}", e)))?;

        let mut data = Vec::new();
        loop {
            let chunk = self
                .vfs
                .read(&handle, data.len() as u64, 65536)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            if chunk.is_empty() {
                self.vfs.close(&handle);
                return Ok(Response::new(ReadFileResponse { data }));
            }
            data.extend_from_slice(&chunk);
        }
    }

    async fn write_file(&self, request: Request<WriteFileRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);

        let flags = sandbox_vfs::OpenFlags {
            read: false,
            write: true,
            create: true,
            truncate: true,
            ..Default::default()
        };
        let handle = self
            .vfs
            .open(&path, flags)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        self.vfs
            .write(&handle, 0, &req.data)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        self.vfs.close(&handle);

        Ok(Response::new(()))
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

        if let Some(expected_hash_pb) = req.expected_root {
            let _expected = ContentHash::try_from(expected_hash_pb)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            // Compare with actual root (placeholder)
        }

        let root = ContentHash::nil();
        let root_pb: sandbox_proto::v1::types::ContentHash = root.into();

        Ok(Response::new(SyncResponse {
            root: Some(root_pb),
        }))
    }
}

pub async fn start_guest_agent(socket_path: &str, vfs: Arc<Vfs>) -> Result<(), SandboxError> {
    let agent = GuestAgent::new(socket_path, vfs).await?;
    agent.run().await
}
