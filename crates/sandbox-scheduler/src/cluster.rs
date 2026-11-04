//! Clustered scheduler for multi-node placement and worker transport.
//!
//! This module extends the base scheduler with:
//! - Multi-node session placement
//! - Worker transport (moving sessions between nodes)
//! - Integration with PgStore for shared state
//! - gRPC communication between scheduler nodes

use crate::placement::{PlacementStrategy, WorkerInfo};
use crate::pools::{IsolatePool, PoolConfig, PoolStats};
use sandbox_types::{
    ErrorCode, SandboxError, SessionConfig, SessionId, SessionState, SessionStatus, TenantId, Tier,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A scheduler node in the cluster.
#[derive(Debug, Clone)]
pub struct SchedulerNode {
    pub id: String,
    pub address: String,
    pub tenant_id: TenantId,
    pub capacity: u64,
    pub current_load: u64,
    pub healthy: bool,
    pub last_heartbeat: std::time::SystemTime,
}

/// Placement decision for a session.
#[derive(Debug, Clone)]
pub struct ClusterPlacementDecision {
    pub session_id: SessionId,
    pub node_id: String,
    pub worker_id: String,
    pub tier: Tier,
}

/// Clustered scheduler that manages sessions across multiple nodes.
#[derive(Debug)]
pub struct ClusterScheduler {
    /// Local placement strategy for this node.
    placement: Arc<RwLock<PlacementStrategy>>,
    /// Local isolate pool.
    isolate_pool: Arc<IsolatePool>,
    /// Workers registered on this node.
    workers: Arc<RwLock<HashMap<String, WorkerInfo>>>,
    /// Sessions managed by this node.
    sessions: Arc<RwLock<HashMap<SessionId, SessionState>>>,
    /// Known scheduler nodes in the cluster.
    cluster_nodes: Arc<RwLock<HashMap<String, SchedulerNode>>>,
    /// This node's ID.
    node_id: String,
    /// This node's address.
    node_address: String,
    /// This node's tenant.
    tenant_id: TenantId,
}

impl ClusterScheduler {
    /// Creates a new clustered scheduler.
    pub fn new(node_id: String, node_address: String, tenant_id: TenantId) -> Self {
        Self {
            placement: Arc::new(RwLock::new(PlacementStrategy::new())),
            isolate_pool: Arc::new(IsolatePool::new(PoolConfig::default())),
            workers: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cluster_nodes: Arc::new(RwLock::new(HashMap::new())),
            node_id,
            node_address,
            tenant_id,
        }
    }

    /// Registers this node in the cluster.
    pub async fn register_self(&self) {
        let node = SchedulerNode {
            id: self.node_id.clone(),
            address: self.node_address.clone(),
            tenant_id: self.tenant_id,
            capacity: 100,
            current_load: 0,
            healthy: true,
            last_heartbeat: std::time::SystemTime::now(),
        };
        self.cluster_nodes
            .write()
            .await
            .insert(self.node_id.clone(), node);
    }

    /// Registers another node in the cluster.
    pub async fn register_node(&self, node: SchedulerNode) {
        self.cluster_nodes
            .write()
            .await
            .insert(node.id.clone(), node);
    }

    /// Gets all healthy nodes in the cluster.
    pub async fn get_healthy_nodes(&self) -> Vec<SchedulerNode> {
        self.cluster_nodes
            .read()
            .await
            .values()
            .filter(|n| n.healthy)
            .cloned()
            .collect()
    }

    /// Creates a session, placing it on the best node in the cluster.
    pub async fn create_session(&self, config: SessionConfig) -> Result<SessionId, SandboxError> {
        let session_id = SessionId::new();

        // First try local placement
        if let Some(isolate) = self.isolate_pool.acquire(&config).await {
            let mut placement = self.placement.write().await;
            let workers = self.workers.read().await;
            let worker_infos: Vec<WorkerInfo> = workers.values().cloned().collect();
            drop(workers);

            if let Some(decision) = placement.place(&config, &worker_infos) {
                self.isolate_pool.release(isolate).await;
                let state = self.create_session_state(session_id, config).await;
                self.sessions.write().await.insert(session_id, state);
                return Ok(decision.session_id);
            }
            self.isolate_pool.release(isolate).await;
        }

        // If local placement fails, try cluster placement
        let nodes = self.get_healthy_nodes().await;
        if nodes.is_empty() {
            return Err(SandboxError::new(
                ErrorCode::Unavailable,
                "no healthy nodes in cluster",
            ));
        }

        // Pick the node with the least load
        let target_node = nodes
            .iter()
            .min_by_key(|n| n.current_load)
            .ok_or_else(|| SandboxError::new(ErrorCode::Unavailable, "no available node"))?;

        // For now, if the target node is local, create locally
        // In a real implementation, this would send an RPC to the target node
        if target_node.id == self.node_id {
            let state = self.create_session_state(session_id, config).await;
            self.sessions.write().await.insert(session_id, state);
            Ok(session_id)
        } else {
            // Would send CreateSession RPC to target node
            Err(SandboxError::new(
                ErrorCode::Unsupported,
                "remote node session creation not yet implemented",
            ))
        }
    }

    /// Creates a session state.
    async fn create_session_state(
        &self,
        session_id: SessionId,
        config: SessionConfig,
    ) -> SessionState {
        SessionState {
            id: session_id,
            tenant_id: config.tenant_id,
            tier: Tier::Isolate,
            allow_vm: config.tier_preference != sandbox_types::TierPreference::Isolate,
            status: SessionStatus::Running,
            config_hash: sandbox_types::hash::ContentHash::nil(),
            vfs_root: None,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            labels: config.labels,
        }
    }

    /// Destroys a session.
    pub async fn destroy_session(&self, session_id: SessionId) -> Result<(), SandboxError> {
        self.sessions.write().await.remove(&session_id);
        Ok(())
    }

    /// Gets a session by ID.
    pub async fn get_session(&self, session_id: SessionId) -> Option<SessionState> {
        self.sessions.read().await.get(&session_id).cloned()
    }

    /// Moves a session to another node (worker transport).
    pub async fn migrate_session(
        &self,
        session_id: SessionId,
        target_node_id: &str,
    ) -> Result<(), SandboxError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| SandboxError::new(ErrorCode::NotFound, "session not found"))?;

        if target_node_id == self.node_id {
            return Ok(()); // Already on this node
        }

        // Update session status
        session.status = SessionStatus::Promoting; // Using Promoting as closest equivalent

        // In a real implementation, this would:
        // 1. Checkpoint the session
        // 2. Send checkpoint to target node via gRPC
        // 3. Target node restores session
        // 4. Update session status and location

        Err(SandboxError::new(
            ErrorCode::Unsupported,
            "session migration not yet implemented",
        ))
    }

    /// Registers a worker on this node.
    pub async fn register_worker(&self, worker: WorkerInfo) {
        self.workers.write().await.insert(worker.id.clone(), worker);
    }

    /// Unregisters a worker.
    pub async fn unregister_worker(&self, worker_id: &str) {
        self.workers.write().await.remove(worker_id);
        self.placement.write().await.release(worker_id);
    }

    /// Gets isolate pool stats.
    pub async fn pool_stats(&self) -> PoolStats {
        self.isolate_pool.stats().await
    }

    /// Gets cluster nodes.
    pub async fn cluster_nodes(&self) -> Vec<SchedulerNode> {
        self.cluster_nodes.read().await.values().cloned().collect()
    }
}

impl Default for ClusterScheduler {
    fn default() -> Self {
        Self::new(
            Uuid::new_v4().to_string(),
            "localhost:50051".to_string(),
            TenantId::new(),
        )
    }
}

/// gRPC service definition for inter-node communication.
// This would be generated from a proto file.
pub mod cluster_rpc {
    // Placeholder for the gRPC service that would be defined in a .proto file
    // service ClusterScheduler {
    //   rpc CreateSession(CreateSessionRequest) returns (CreateSessionResponse);
    //   rpc MigrateSession(MigrateSessionRequest) returns (MigrateSessionResponse);
    //   rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
    // }
}
