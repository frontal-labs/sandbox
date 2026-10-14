use crate::client::ExecResult;
use crate::Client;
use sandbox_types::{SandboxError, SessionConfig, SessionId, SnapshotId};

pub struct EmbeddedSandbox {
    client: Client,
    session_id: Option<SessionId>,
}

impl EmbeddedSandbox {
    pub async fn new(config: SessionConfig) -> Result<Self, SandboxError> {
        let client = Client::embedded()?;
        let session_id = client.create_session(config).await?;
        Ok(Self {
            client,
            session_id: Some(session_id),
        })
    }

    pub async fn exec(&self, command: &str) -> Result<ExecResult, SandboxError> {
        let session_id = self
            .session_id
            .ok_or_else(|| SandboxError::invalid_state("No active session"))?;

        self.client.exec(session_id, command).await
    }

    pub async fn snapshot(&self) -> Result<SnapshotId, SandboxError> {
        let session_id = self
            .session_id
            .ok_or_else(|| SandboxError::invalid_state("No active session"))?;

        self.client.snapshot(session_id).await
    }

    pub async fn destroy(&self) -> Result<(), SandboxError> {
        if let Some(session_id) = self.session_id {
            self.client.destroy_session(session_id).await?;
        }
        Ok(())
    }
}
