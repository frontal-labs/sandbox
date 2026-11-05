//! Each chaos scenario has its own test binary: the durable store is per process.
//! Failure tests for the durable server. Each one injects a fault and checks what the server
//! promises afterwards: a write it acknowledged is still there after a crash, and a damaged
//! state file is refused rather than read as an empty server.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tower::ServiceExt;

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sandbox-chaos-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn config() -> Value {
    serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap()
}

async fn call(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Body,
) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

async fn create(app: &axum::Router, key: &str) -> String {
    let (status, body) = call(
        app,
        key,
        "POST",
        "/v1/sessions",
        Body::from(config().to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["session_id"].as_str().unwrap().to_string()
}

/// Several sessions take writes at once. The process "crashes" partway through, with no
/// shutdown. Every write the server acknowledged before the crash must read back after a restart.
#[tokio::test]
async fn acknowledged_writes_survive_a_crash_under_concurrent_load() {
    let dir = fresh_dir("crash");
    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");

    let mut sessions = Vec::new();
    for _ in 0..4 {
        sessions.push(create(&app, &key).await);
    }

    // Every acknowledgement is recorded as it arrives, so the check below covers all of them,
    // including those written before the crash cut the writers off.
    let acknowledged: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut tasks = Vec::new();
    for (index, id) in sessions.iter().cloned().enumerate() {
        let app = app.clone();
        let key = key.clone();
        let acknowledged = acknowledged.clone();
        tasks.push(tokio::spawn(async move {
            for version in 1..=400u32 {
                let uri = format!("/v1/sessions/{id}/fs/log-{index}.txt");
                let (status, _) = call(
                    &app,
                    &key,
                    "PUT",
                    &uri,
                    Body::from(format!("version {version}")),
                )
                .await;
                if status == StatusCode::NO_CONTENT {
                    acknowledged.lock().unwrap().insert(id.clone(), version);
                }
            }
        }));
    }

    // The crash comes while the writers are still going.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    for task in &tasks {
        task.abort();
    }
    drop(app);

    let acked: HashMap<String, u32> = acknowledged.lock().unwrap().clone();
    assert!(
        !acked.is_empty(),
        "the crash must come after some acknowledged writes"
    );

    let (restarted, _) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    for (index, id) in sessions.iter().enumerate() {
        let Some(&version) = acked.get(id) else {
            continue;
        };
        let (status, body) = call(
            &restarted,
            &key,
            "GET",
            &format!("/v1/sessions/{id}/fs/log-{index}.txt"),
            Body::empty(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "session {id}: {}",
            String::from_utf8_lossy(&body)
        );
        let read: u32 = String::from_utf8(body)
            .unwrap()
            .trim_start_matches("version ")
            .trim()
            .parse()
            .unwrap();
        assert!(
            read >= version,
            "session {id}: acknowledged version {version}, read back {read}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
