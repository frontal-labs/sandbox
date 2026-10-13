use sandbox_types::{SessionConfig, SandboxError};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct VmPool {
    available: Arc<Mutex<VecDeque<VmHandle>>>,
    config: PoolConfig,
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub min_size: usize,
    pub max_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VmHandle {
    pub id: String,
    pub image: String,
}

impl VmPool {
    pub fn new(config: PoolConfig) -> Self {
        Self {
            available: Arc::new(Mutex::new(VecDeque::new())),
            config,
        }
    }

    pub async fn acquire(&self, _config: &SessionConfig) -> Result<Option<VmHandle>, SandboxError> {
        Ok(self.available.lock().await.pop_front())
    }

    pub async fn release(&self, handle: VmHandle) {
        self.available.lock().await.push_back(handle);
    }

    pub async fn stats(&self) -> PoolStats {
        let available = self.available.lock().await;
        PoolStats {
            available: available.len(),
            max_size: self.config.max_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub available: usize,
    pub max_size: usize,
}