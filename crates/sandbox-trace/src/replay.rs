use sandbox_types::Event;
use sandbox_types::SandboxError;

#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub deterministic: bool,
    pub virtual_clock: bool,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            deterministic: true,
            virtual_clock: true,
        }
    }
}

pub struct ReplayEngine {}

impl ReplayEngine {
    pub fn new(_config: ReplayConfig) -> Self {
        Self {}
    }

    pub async fn replay(
        &self,
        _events: Vec<Event>,
        _from_snapshot: Option<String>,
    ) -> Result<ReplayResult, SandboxError> {
        Ok(ReplayResult {
            divergences: Vec::new(),
            completed: true,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReplayResult {
    pub divergences: Vec<String>,
    pub completed: bool,
}
