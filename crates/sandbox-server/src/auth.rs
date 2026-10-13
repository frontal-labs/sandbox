use sandbox_types::SandboxError;

pub struct AuthManager;

impl AuthManager {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_api_key(&self, _api_key: &str) -> Result<(), SandboxError> {
        // TODO: Implement API key validation
        Ok(())
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}