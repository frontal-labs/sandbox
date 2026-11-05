//! A sleeping session on a durable server is parked on disk: its record is one file in
//! `sessions/`, and it holds no table entry. These tests run one server against one data
//! directory, in one process, because the blob store is process-wide.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

fn config() -> Value {
    json!({
        "tenant_id": "00000000000000000000000000",
        "tier_preference": "isolate",
        "limits": {},
        "network": {"default_deny": true, "rules": []},
        "env": {},
        "cwd": "/work",
        "labels": {},
    })
}

fn parked_file(dir: &Path, id: &str) -> PathBuf {
    dir.join("sessions").join(format!("{id}.json"))
}

async fn create(app: &axum::Router, key: &str) -> String {
    let (status, body) = call(app, "POST", "/v1/sessions", key, Some(config())).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_sleeping_session_is_parked_on_disk_and_comes_back_on_wake() {
    let dir: PathBuf = std::env::temp_dir().join(format!("sandbox-parked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");

    let id = create(&app, &key).await;
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        Some(json!({"argv": ["echo", "before"], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/keep.txt"),
        &key,
        None,
    )
    .await;
    assert!(
        status.is_success(),
        "write failed: {}",
        String::from_utf8_lossy(&body)
    );

    // Sleep parks the session: its record is a file, and the table no longer holds it.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/sleep"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let snapshot_id = serde_json::from_slice::<Value>(&body).unwrap()["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        parked_file(&dir, &id).exists(),
        "a sleeping session has a parked record"
    );

    // A sleeping session is still visible, with its trace, and it is still asleep.
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "sleeping"
    );
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}/trace"), &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("'echo' 'before'"));

    // A restart keeps it parked, and a request wakes it with its files.
    drop(app);
    let (app, none) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(none.is_none());
    assert!(
        parked_file(&dir, &id).exists(),
        "the record survives a restart"
    );
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/keep.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(
        !parked_file(&dir, &id).exists(),
        "a woken session leaves no parked record"
    );
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "running"
    );

    // The sleep's snapshot was written as a file before the restart, and it is still listed.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/snapshots"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let listed = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["id"], snapshot_id.as_str());

    // Destroying a parked session removes its record, and it is gone.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/sleep"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(parked_file(&dir, &id).exists());
    let (status, _) = call(&app, "DELETE", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!parked_file(&dir, &id).exists());
    let (status, _) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(&dir);
}
