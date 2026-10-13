use sandbox_types::{SandboxError, SessionConfig};
use sandbox_session::Session;
use std::sync::Arc;

pub struct PromoteManager;

impl PromoteManager {
    pub async fn promote_session(
        _session: Arc<dyn Session>,
        _config: SessionConfig,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::unsupported("promotion not implemented"))
    }
}

impl Default for PromoteManager {
    fn default() -> Self {
        Self
    }
}