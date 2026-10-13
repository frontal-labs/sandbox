use sandbox_types::{SessionConfig, SessionId, SnapshotId, SandboxError};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

#[derive(Debug)]
struct ClientInner {
    base_url: String,
}

impl Client {
    pub fn embedded() -> Result<Self, SandboxError> {
        Ok(Self {
            inner: Arc::new(ClientInner {
                base_url: "embedded://".to_string(),
            }),
        })
    }

    pub fn from_env() -> Result<Self, SandboxError> {
        let _api_key = std::env::var("SANDBOX_API_KEY").ok();
        let base_url = std::env::var("SANDBOX_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
        
        Ok(Self {
            inner: Arc::new(ClientInner {
                base_url,
            }),
        })
    }

    pub async fn create_session(&self, _config: SessionConfig) -> Result<SessionId, SandboxError> {
        if self.inner.base_url == "embedded://" {
            return Ok(SessionId::new());
        }
        
        Err(SandboxError::unsupported("HTTP client not implemented"))
    }

    pub async fn exec(&self, _session_id: SessionId, _command: &str) -> Result<ExecResult, SandboxError> {
        if self.inner.base_url == "embedded://" {
            return Ok(ExecResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        
        Err(SandboxError::unsupported("HTTP client not implemented"))
    }

    pub async fn snapshot(&self, _session_id: SessionId) -> Result<SnapshotId, SandboxError> {
        Err(SandboxError::unsupported("Not implemented"))
    }

    pub async fn destroy_session(&self, _session_id: SessionId) -> Result<(), SandboxError> {
        Err(SandboxError::unsupported("Not implemented"))
    }
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}