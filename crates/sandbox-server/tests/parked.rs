//! Unit tests moved out of src/parked.rs.

use sandbox_server::parked::parked_store;
use sandbox_server::persist::SavedSession;
use sandbox_types::hash::ContentHash;
use sandbox_types::{CheckpointPolicy, SessionId, SessionState, SessionStatus, TenantId, Tier};
use std::collections::HashMap;
use std::path::PathBuf;
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
    let store = parked_store(&data).unwrap();
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
    let store = parked_store(&data).unwrap();
    let id = SessionId::new();
    std::fs::write(store.path(id), b"{not json").unwrap();
    assert!(store.get(id).await.is_err());
    let _ = std::fs::remove_dir_all(&data);
}

#[tokio::test]
async fn listing_skips_temporary_and_foreign_files() {
    let data = dir("listing");
    let store = parked_store(&data).unwrap();
    let id = SessionId::new();
    store.put(id, &record("key-a")).await.unwrap();
    std::fs::write(data.join("sessions").join("half-written.tmp"), b"x").unwrap();
    std::fs::write(data.join("sessions").join("notes.txt"), b"x").unwrap();
    assert_eq!(store.ids().await.unwrap(), vec![id]);
    let _ = std::fs::remove_dir_all(&data);
}
