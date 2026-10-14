use sandbox_types::{SessionConfig, SessionId, TenantId};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PlacementDecision {
    pub worker_id: String,
    pub session_id: SessionId,
}

#[derive(Debug)]
pub struct PlacementStrategy {
    worker_loads: HashMap<String, u64>,
}

impl PlacementStrategy {
    pub fn new() -> Self {
        Self {
            worker_loads: HashMap::new(),
        }
    }

    pub fn place(
        &mut self,
        _config: &SessionConfig,
        workers: &[WorkerInfo],
    ) -> Option<PlacementDecision> {
        workers
            .iter()
            .min_by_key(|w| self.worker_loads.get(&w.id).copied().unwrap_or(0))
            .map(|w| {
                *self.worker_loads.entry(w.id.clone()).or_insert(0) += 1;
                PlacementDecision {
                    worker_id: w.id.clone(),
                    session_id: SessionId::new(),
                }
            })
    }

    pub fn release(&mut self, worker_id: &str) {
        if let Some(load) = self.worker_loads.get_mut(worker_id) {
            *load = load.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: String,
    pub tenant_id: TenantId,
    pub capacity: u64,
    pub current_load: u64,
}

impl Default for PlacementStrategy {
    fn default() -> Self {
        Self::new()
    }
}
