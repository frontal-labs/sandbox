//! A one-session convenience wrapper over [`Client`]. It talks to the server
//! like any other client, so it needs a running `sandbox-server`.

use crate::client::{Client, ExecResult};
use sandbox_types::{SandboxError, SessionConfig, SessionId, SnapshotId};

pub struct EmbeddedSandbox {
    client: Client,
    session_id: SessionId,
}

impl EmbeddedSandbox {
    /// Creates a session on the server the client points at.
    pub async fn new(client: Client, config: SessionConfig) -> Result<Self, SandboxError> {
        let session_id = client.create_session(config).await?;
        Ok(Self { client, session_id })
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub async fn exec(&self, command: &str) -> Result<ExecResult, SandboxError> {
        self.client.exec(self.session_id, command).await
    }

    pub async fn snapshot(&self) -> Result<SnapshotId, SandboxError> {
        self.client.snapshot(self.session_id).await
    }

    pub async fn destroy(&self) -> Result<(), SandboxError> {
        self.client.destroy_session(self.session_id).await
    }
}
