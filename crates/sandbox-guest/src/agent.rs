use sandbox_types::SandboxError;
use tokio::net::UnixListener;
use tracing::{error, info};

pub struct GuestAgent {
    listener: UnixListener,
}

impl GuestAgent {
    pub async fn new(socket_path: &str) -> Result<Self, SandboxError> {
        let listener = UnixListener::bind(socket_path).map_err(|e| {
            SandboxError::internal(format!("Failed to bind to {}: {}", socket_path, e))
        })?;
        Ok(Self { listener })
    }

    pub async fn run(&self) -> Result<(), SandboxError> {
        info!("Guest agent listening on socket");

        loop {
            let (stream, _addr) = self.listener.accept().await.map_err(|e| {
                SandboxError::internal(format!("Failed to accept connection: {}", e))
            })?;

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream).await {
                    error!("Connection error: {}", e);
                }
            });
        }
    }

    async fn handle_connection(_stream: tokio::net::UnixStream) -> Result<(), SandboxError> {
        // Handle guest protocol messages
        Ok(())
    }
}
