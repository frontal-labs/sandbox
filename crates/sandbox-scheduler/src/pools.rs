use sandbox_types::{SessionConfig, SessionId, Tier};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub min_size: usize,
    pub max_size: usize,
    pub idle_timeout_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
            idle_timeout_secs: 300,
        }
    }
}

#[derive(Debug)]
pub struct IsolatePool {
    config: PoolConfig,
    available: RwLock<Vec<PooledIsolate>>,
    in_use: RwLock<HashMap<SessionId, PooledIsolate>>,
}

#[derive(Debug, Clone)]
pub struct PooledIsolate {
    pub id: String,
    pub tier: Tier,
    pub created_at: std::time::Instant,
    pub last_used: std::time::Instant,
}

impl IsolatePool {
    pub fn new(config: PoolConfig) -> Self {
        Self {
            config,
            available: RwLock::new(Vec::new()),
            in_use: RwLock::new(HashMap::new()),
        }
    }

    pub async fn acquire(&self, _config: &SessionConfig) -> Option<PooledIsolate> {
        let mut available = self.available.write().await;
        available.pop()
    }

    pub async fn release(&self, isolate: PooledIsolate) {
        let mut available = self.available.write().await;
        if available.len() < self.config.max_size {
            available.push(isolate);
        }
    }

    pub async fn stats(&self) -> PoolStats {
        let available = self.available.read().await;
        let in_use = self.in_use.read().await;
        PoolStats {
            available: available.len(),
            in_use: in_use.len(),
            max_size: self.config.max_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub available: usize,
    pub in_use: usize,
    pub max_size: usize,
}
