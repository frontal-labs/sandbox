use sandbox_types::{SandboxError, SessionConfig, SessionId};

pub struct Worker {
}

impl Worker {
    pub async fn new(_config: SessionConfig) -> Result<Self, SandboxError> {
        Ok(Self {})
    }
    
    pub async fn spawn_session(&self, _config: SessionConfig) -> Result<SessionId, SandboxError> {
        Err(SandboxError::unsupported("worker not implemented"))
    }
    
    pub async fn shutdown(&self) -> Result<(), SandboxError> {
        Ok(())
    }
}

impl Default for Worker {
    fn default() -> Self {
        Self {}
    }
}