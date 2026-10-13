use crate::placement::{PlacementStrategy, WorkerInfo};
use crate::pools::{IsolatePool, PoolConfig, PoolStats};
use sandbox_types::{SessionConfig, SessionId, SessionState, Tier, SessionStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct Scheduler {
    placement: Arc<RwLock<PlacementStrategy>>,
    isolate_pool: Arc<IsolatePool>,
    workers: Arc<RwLock<HashMap<String, WorkerInfo>>>,
    sessions: Arc<RwLock<HashMap<SessionId, SessionState>>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            placement: Arc::new(RwLock::new(PlacementStrategy::new())),
            isolate_pool: Arc::new(IsolatePool::new(PoolConfig::default())),
            workers: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(&self, config: SessionConfig) -> Result<SessionId, sandbox_types::SandboxError> {
        let session_id = SessionId::new();
        
        let labels = config.labels.clone();
        
        let state = SessionState {
            id: session_id,
            tenant_id: config.tenant_id,
            tier: Tier::Isolate,
            status: SessionStatus::Running,
            config_hash: sandbox_types::hash::ContentHash::nil(),
            vfs_root: None,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            labels,
        };

        self.sessions.write().await.insert(session_id, state);
        
        if let Some(isolate) = self.isolate_pool.acquire(&config).await {
            // Use the pooled isolate
            let mut placement = self.placement.write().await;
            let workers = self.workers.read().await;
            let worker_infos: Vec<WorkerInfo> = workers.values().cloned().collect();
            drop(workers);
            
            if let Some(decision) = placement.place(&config, &worker_infos) {
                self.isolate_pool.release(isolate).await;
                return Ok(decision.session_id);
            }
            self.isolate_pool.release(isolate).await;
        }

        Ok(session_id)
    }

    pub async fn destroy_session(&self, session_id: SessionId) -> Result<(), sandbox_types::SandboxError> {
        self.sessions.write().await.remove(&session_id);
        Ok(())
    }

    pub async fn get_session(&self, session_id: SessionId) -> Option<SessionState> {
        self.sessions.read().await.get(&session_id).cloned()
    }

    pub async fn register_worker(&self, worker: WorkerInfo) {
        self.workers.write().await.insert(worker.id.clone(), worker);
    }

    pub async fn unregister_worker(&self, worker_id: &str) {
        self.workers.write().await.remove(worker_id);
        self.placement.write().await.release(worker_id);
    }

    pub async fn pool_stats(&self) -> PoolStats {
        self.isolate_pool.stats().await
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}