use sandbox_types::{SessionConfig, SandboxError};
use std::net::SocketAddr;

pub struct VmmDriver;

impl VmmDriver {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_vm(&self, _config: &SessionConfig) -> Result<VmHandle, SandboxError> {
        Err(SandboxError::unsupported("VMM not implemented"))
    }
}

#[derive(Debug, Clone)]
pub struct VmHandle {
    pub id: String,
    pub vsock_addr: SocketAddr,
}

impl Default for VmmDriver {
    fn default() -> Self {
        Self::new()
    }
}