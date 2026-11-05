//! Sleeping sessions, parked on disk. A parked session holds no memory in the server:
//! its record is one file in `sessions/`, named by its ID, and it is read back only when a
//! request needs it. Waking it removes the file; sleeping it writes one. The file holds the
//! same record the state log uses for a session (`SavedSession`), so the format is unchanged.

use crate::persist::{create_private_dir, write_file_atomic, SavedSession};
use sandbox_types::{SandboxError, SessionId};
use std::path::{Path, PathBuf};

const DIR: &str = "sessions";

/// The directory of parked sessions in a data directory.
#[derive(Debug, Clone)]
pub(crate) struct ParkedStore {
    dir: PathBuf,
}

impl ParkedStore {
    /// Opens (and creates, if needed) the parked-session directory in `data_dir`.
    pub(crate) fn open(data_dir: &Path) -> Result<Self, SandboxError> {
        let dir = data_dir.join(DIR);
        create_private_dir(&dir)
            .map_err(|e| SandboxError::internal(format!("cannot create {}: {e}", dir.display())))?;
        Ok(Self { dir })
    }

    fn path(&self, id: SessionId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Writes a parked session's record, replacing any earlier one.
    pub(crate) async fn put(
        &self,
        id: SessionId,
        record: &SavedSession,
    ) -> Result<(), SandboxError> {
        let bytes = serde_json::to_vec(record).map_err(|e| {
            SandboxError::internal(format!("cannot encode a sleeping session: {e}"))
        })?;
        write_file_atomic(self.path(id), bytes).await
    }

    /// Reads a parked session's record, or `None` when the session is not parked.
    pub(crate) async fn get(&self, id: SessionId) -> Result<Option<SavedSession>, SandboxError> {
        let bytes = match tokio::fs::read(self.path(id)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(SandboxError::internal(format!(
                    "cannot read a sleeping session: {e}"
                )))
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            SandboxError::internal(format!("a sleeping session's record is damaged: {e}"))
        })
    }

    /// Removes a parked session. Returns whether a record was there.
    pub(crate) async fn remove(&self, id: SessionId) -> Result<bool, SandboxError> {
        match tokio::fs::remove_file(self.path(id)).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(SandboxError::internal(format!(
                "cannot remove a sleeping session: {e}"
            ))),
        }
    }

    /// Every parked session's ID. The records are not read.
    pub(crate) async fn ids(&self) -> Result<Vec<SessionId>, SandboxError> {
        let list_error = |e: std::io::Error| {
            SandboxError::internal(format!("cannot list sleeping sessions: {e}"))
        };
        let mut entries = tokio::fs::read_dir(&self.dir).await.map_err(list_error)?;
        let mut ids = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(list_error)? {
            let name = entry.file_name();
            if let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".json")) {
                let id: SessionId = stem.parse().map_err(|_| {
                    SandboxError::internal(format!(
                        "an unexpected file in the sessions directory: {stem}"
                    ))
                })?;
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_types::hash::ContentHash;
    use sandbox_types::{CheckpointPolicy, SessionState, SessionStatus, TenantId, Tier};
    use std::collections::HashMap;
    use std::time::SystemTime;

    fn record(owner: &str) -> SavedSession {
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        SavedSession {
            owner: owner.to_string(),
            state: SessionState {
                id: SessionId::new(),
                tenant_id: TenantId::nil(),
                tier: Tier::Isolate,
                allow_vm: false,
                status: SessionStatus::Sleeping,
                config_hash: ContentHash::nil(),
                vfs_root: None,
                created_at: now,
                updated_at: now,
                labels: HashMap::new(),
            },
            root: ContentHash::nil(),
            cwd: "/".to_string(),
            env: HashMap::new(),
            sleeping: true,
            limits: Default::default(),
            trace: String::new(),
            trace_complete: true,
            checkpoint_policy: CheckpointPolicy::default(),
            checkpoint_retain: sandbox_types::session::DEFAULT_CHECKPOINT_RETAIN,
            roots: Vec::new(),
            spilled: Vec::new(),
        }
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("parked-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn a_parked_record_round_trips_and_removes_cleanly() {
        let data = dir("round-trip");
        let store = ParkedStore::open(&data).unwrap();
        let id = SessionId::new();
        assert!(store.get(id).await.unwrap().is_none());

        store.put(id, &record("key-a")).await.unwrap();
        let back = store.get(id).await.unwrap().unwrap();
        assert_eq!(back.owner, "key-a");
        assert_eq!(store.ids().await.unwrap(), vec![id]);

        assert!(store.remove(id).await.unwrap());
        assert!(!store.remove(id).await.unwrap());
        assert!(store.get(id).await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn a_damaged_record_is_refused_not_ignored() {
        let data = dir("damaged");
        let store = ParkedStore::open(&data).unwrap();
        let id = SessionId::new();
        std::fs::write(store.path(id), b"{not json").unwrap();
        assert!(store.get(id).await.is_err());
        let _ = std::fs::remove_dir_all(&data);
    }
}
