//! Unit tests moved out of src/persist.rs.

use sandbox_server::persist::{
    log_name, Saved, SavedKey, SavedSession, Store, COMPACT_FLOOR, FILE, READABLE, VERSION,
};
use sandbox_types::hash::ContentHash;
use sandbox_types::{SessionId, SessionState, SessionStatus, TenantId, Tier};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sandbox-persist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn version_one_state_still_loads_its_sleeping_sessions() {
    let v1 = r#"{"version":1,"keys":[],"snapshots":[],"sleeping":[]}"#;
    let saved: Saved = serde_json::from_str(v1).unwrap();
    assert_eq!(saved.version, 1);
    assert!(saved.sessions.is_empty());
    assert!(READABLE.contains(&saved.version));
}

#[test]
fn a_version_two_session_keeps_its_shell_state() {
    let state = SessionState {
        id: SessionId::new(),
        tenant_id: TenantId::nil(),
        tier: Tier::Isolate,
        allow_vm: false,
        status: SessionStatus::Running,
        config_hash: ContentHash::nil(),
        vfs_root: None,
        created_at: SystemTime::now(),
        updated_at: SystemTime::now(),
        labels: HashMap::new(),
    };
    let session = SavedSession {
        owner: "key".to_string(),
        state,
        root: ContentHash::nil(),
        cwd: "/work".to_string(),
        env: HashMap::from([("GREETING".to_string(), "hello".to_string())]),
        sleeping: false,
        limits: Default::default(),
        trace: String::new(),
        trace_complete: true,
        checkpoint_policy: Default::default(),
        checkpoint_retain: 50,
        roots: Vec::new(),
        spilled: Vec::new(),
    };
    let saved = Saved {
        version: VERSION,
        keys: Vec::new(),
        snapshots: Vec::new(),
        templates: Vec::new(),
        sleeping: Vec::new(),
        sessions: vec![session],
        log_epoch: 0,
    };
    let text = serde_json::to_string(&saved).unwrap();
    let back: Saved = serde_json::from_str(&text).unwrap();
    assert_eq!(back.sessions[0].cwd, "/work");
    assert_eq!(back.sessions[0].env["GREETING"], "hello");
}

#[test]
fn an_unknown_state_version_is_refused() {
    let d = dir("version");
    std::fs::write(d.join(FILE), r#"{"version":9,"keys":[],"snapshots":[]}"#).unwrap();
    let err = Store::open(&d).err().expect("version 9 must be refused");
    assert!(err.message.contains("version 9"), "{}", err.message);
    let _ = std::fs::remove_dir_all(&d);
}

fn saved_session(id: SessionId, label: &str) -> SavedSession {
    // A fixed time, so a session that did not change saves the same bytes.
    let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    SavedSession {
        owner: "key".to_string(),
        state: SessionState {
            id,
            tenant_id: TenantId::nil(),
            tier: Tier::Isolate,
            allow_vm: false,
            status: SessionStatus::Running,
            config_hash: ContentHash::nil(),
            vfs_root: None,
            created_at: now,
            updated_at: now,
            labels: HashMap::from([("name".to_string(), label.to_string())]),
        },
        root: ContentHash::nil(),
        cwd: "/work".to_string(),
        env: HashMap::new(),
        sleeping: false,
        limits: Default::default(),
        trace: String::new(),
        trace_complete: true,
        checkpoint_policy: Default::default(),
        checkpoint_retain: 50,
        roots: Vec::new(),
        spilled: Vec::new(),
    }
}

fn state_of(sessions: Vec<SavedSession>) -> Saved {
    Saved {
        version: VERSION,
        keys: vec![SavedKey {
            id: "k1".to_string(),
            digest: "digest".to_string(),
            tenant: TenantId::nil(),
        }],
        snapshots: Vec::new(),
        templates: Vec::new(),
        sleeping: Vec::new(),
        sessions,
        log_epoch: 0,
    }
}

async fn save(store: &Store, saved: Saved) {
    store.save(move || async move { Ok(saved) }).await.unwrap();
}

fn label_of(session: &SavedSession) -> &str {
    &session.state.labels["name"]
}

#[tokio::test]
async fn saves_append_only_what_changed() {
    let d = dir("append");
    let (store, _) = Store::load(&d).unwrap();
    let (a, b) = (SessionId::new(), SessionId::new());
    save(
        &store,
        state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
    )
    .await;
    let log = d.join(log_name(0));
    let first = std::fs::metadata(&log).unwrap().len();

    // Nothing changed, so nothing is appended.
    save(
        &store,
        state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
    )
    .await;
    assert_eq!(std::fs::metadata(&log).unwrap().len(), first);

    // One session changed, so one record is appended, and it is for that session.
    save(
        &store,
        state_of(vec![saved_session(a, "changed"), saved_session(b, "b")]),
    )
    .await;
    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "a key and two sessions, then one more session"
    );
    assert!(lines[3].contains("put_session") && lines[3].contains("changed"));
    assert!(!d.join(FILE).exists(), "no base file until a compaction");
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn replay_restores_the_last_state_including_deletions() {
    let d = dir("replay");
    let (a, b) = (SessionId::new(), SessionId::new());
    {
        let (store, _) = Store::load(&d).unwrap();
        save(
            &store,
            state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
        )
        .await;
        save(&store, state_of(vec![saved_session(a, "changed")])).await;
    }
    let (_, saved) = Store::load(&d).unwrap();
    assert_eq!(saved.sessions.len(), 1, "the removed session stays removed");
    assert_eq!(saved.sessions[0].state.id, a);
    assert_eq!(label_of(&saved.sessions[0]), "changed");
    assert_eq!(saved.keys.len(), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_torn_final_record_is_dropped_and_truncated() {
    let d = dir("torn");
    let a = SessionId::new();
    {
        let (store, _) = Store::load(&d).unwrap();
        save(&store, state_of(vec![saved_session(a, "a")])).await;
    }
    let log = d.join(log_name(0));
    let complete = std::fs::metadata(&log).unwrap().len();
    // A crash mid-append leaves a record with no newline.
    let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    std::io::Write::write_all(&mut file, br#"{"op":"put_sess"#).unwrap();
    drop(file);

    let (store, saved) = Store::load(&d).unwrap();
    assert_eq!(saved.sessions.len(), 1);
    assert_eq!(
        std::fs::metadata(&log).unwrap().len(),
        complete,
        "truncated to the last record"
    );

    // The next save appends on a clean line, and it replays.
    save(&store, state_of(vec![saved_session(a, "after")])).await;
    let (_, saved) = Store::load(&d).unwrap();
    assert_eq!(label_of(&saved.sessions[0]), "after");
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn compaction_writes_a_new_base_and_starts_a_new_log() {
    let d = dir("compact");
    let a = SessionId::new();
    let big = "x".repeat(COMPACT_FLOOR as usize);
    {
        let (store, _) = Store::load(&d).unwrap();
        save(&store, state_of(vec![saved_session(a, &big)])).await;
    }
    // The first save was larger than the floor, so it compacted.
    assert!(d.join(FILE).exists(), "a base file was written");
    assert!(!d.join(log_name(0)).exists(), "the old log is gone");
    assert_eq!(std::fs::metadata(d.join(log_name(1))).unwrap().len(), 0);
    let (store, saved) = Store::load(&d).unwrap();
    assert_eq!(saved.log_epoch, 1);
    assert_eq!(label_of(&saved.sessions[0]).len(), big.len());

    // A later change goes to the new log, and both halves replay.
    save(&store, state_of(vec![saved_session(a, "small")])).await;
    assert!(std::fs::metadata(d.join(log_name(1))).unwrap().len() > 0);
    let (_, saved) = Store::load(&d).unwrap();
    assert_eq!(label_of(&saved.sessions[0]), "small");
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_damaged_record_before_the_end_is_refused() {
    let d = dir("damaged");
    let (a, b) = (SessionId::new(), SessionId::new());
    {
        let (store, _) = Store::load(&d).unwrap();
        save(
            &store,
            state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
        )
        .await;
    }
    let log = d.join(log_name(0));
    let text = std::fs::read_to_string(&log).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines[1] = "not a record";
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let err = Store::load(&d)
        .err()
        .expect("a damaged record must be refused");
    assert!(err.message.contains("damaged"), "{}", err.message);
    let _ = std::fs::remove_dir_all(&d);
}
