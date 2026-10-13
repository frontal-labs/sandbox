use sandbox_types::{Snapshot, SnapshotId, SessionState, SandboxError, ids::SessionId, ids::TenantId, session::SessionStatus};
use sandbox_types::hash::ContentHash;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SnapshotManager {
    snapshots: Arc<RwLock<HashMap<SnapshotId, Snapshot>>>,
}

impl SnapshotManager {
    pub fn new() -> Self {
        Self {
            snapshots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn take_snapshot(
        &self,
        session: &SessionState,
        vfs_root: ContentHash,
    ) -> Result<SnapshotId, SandboxError> {
        let snapshot_id = SnapshotId::new();
        let snapshot = Snapshot {
            id: snapshot_id,
            parent: None,
            vfs_root,
            env: HashMap::new(),
            cwd: "/work".to_string(),
            tier: session.tier,
            runtime_blob: None,
            created_at: std::time::SystemTime::now(),
            labels: session.labels.clone(),
        };

        self.snapshots.write().await.insert(snapshot_id, snapshot);
        Ok(snapshot_id)
    }

    pub async fn restore_snapshot(&self, snapshot_id: SnapshotId) -> Result<Snapshot, SandboxError> {
        self.snapshots
            .read()
            .await
            .get(&snapshot_id)
            .cloned()
            .ok_or_else(|| SandboxError::not_found(format!("Snapshot not found: {}", snapshot_id)))
    }

    pub async fn fork_snapshot(
        &self,
        snapshot_id: SnapshotId,
        count: usize,
    ) -> Result<Vec<SessionState>, SandboxError> {
        let snapshot = self.restore_snapshot(snapshot_id).await?;
        
        let mut sessions = Vec::new();
        for _ in 0..count {
            let session = SessionState {
                id: SessionId::new(),
                tenant_id: TenantId::nil(),
                tier: snapshot.tier,
                status: SessionStatus::Running,
                config_hash: snapshot.vfs_root,
                vfs_root: Some(snapshot.vfs_root),
                created_at: std::time::SystemTime::now(),
                updated_at: std::time::SystemTime::now(),
                labels: snapshot.labels.clone(),
            };
            sessions.push(session);
        }
        
        Ok(sessions)
    }
}

impl Default for SnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}