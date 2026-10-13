use sandbox_types::{SessionConfig, SandboxError};

pub struct Jailer;

impl Jailer {
    pub fn new() -> Self {
        Self
    }

    pub async fn setup(&self, _config: &SessionConfig) -> Result<(), SandboxError> {
        Err(SandboxError::unsupported("Jailer not implemented"))
    }
}

impl Default for Jailer {
    fn default() -> Self {
        Self::new()
    }
}