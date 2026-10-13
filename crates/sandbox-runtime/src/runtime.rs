use sandbox_types::SandboxError;
use deno_core::{JsRuntime as DenoJsRuntime, RuntimeOptions};

pub struct Runtime {
}

impl Runtime {
    pub fn new() -> Result<Self, SandboxError> {
        let _runtime = DenoJsRuntime::new(RuntimeOptions::default());
        Ok(Self {})
    }

    pub async fn execute_script(&mut self, _code: &str) -> Result<String, SandboxError> {
        // TODO: Implement proper script execution
        Err(SandboxError::unsupported("Script execution not fully implemented"))
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new().expect("Failed to create Runtime")
    }
}