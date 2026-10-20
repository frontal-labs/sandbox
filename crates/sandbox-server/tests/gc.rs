//! Garbage collection of the blob store, driven through the real router.
//!
//! This is its own test binary, because the blob store is shared by the whole
//! process. A collection in one test could sweep blobs another test is using, so
//! every test here takes `SERIAL` first.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router, create_router_with_state};
use sandbox_session::blob_store;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// Serializes the tests in this binary, which share one blob store.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    body: Option<(&str, Vec<u8>)>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some((content_type, bytes)) => {
            builder = builder.header(header::CONTENT_TYPE, content_type);
            Body::from(bytes)
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

fn json_body(value: &Value) -> Option<(&'static str, Vec<u8>)> {
    Some(("application/json", serde_json::to_vec(value).unwrap()))
}

async fn create(app: &axum::Router, key: &str) -> String {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(app, "POST", "/v1/sessions", key, json_body(&config)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["session_id"].as_str().unwrap().to_string()
}

async fn exec(app: &axum::Router, key: &str, id: &str, argv: &[&str]) {
    let request = json!({
        "argv": argv, "env": {}, "cwd": null, "stdin": null,
        "timeout_ms": null, "stream": false
    });
    let (status, body) = call(
        app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        key,
        json_body(&request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn collection_removes_unreachable_blobs_and_keeps_what_sessions_and_snapshots_need() {
    let _serial = SERIAL.lock().await;
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);

    let id = create(&app, &key).await;
    exec(&app, &key, &id, &["echo", "one"]).await;
    // Write a file, then remove it. Its contents become unreachable.
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/keep.txt"),
        &key,
        Some(("application/octet-stream", b"kept bytes".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/gone.txt"),
        &key,
        Some(("application/octet-stream", b"transient bytes".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    exec(&app, &key, &id, &["rm", "/gone.txt"]).await;

    // A snapshot keeps its own files reachable.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        &key,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let snapshot_id = serde_json::from_slice::<Value>(&body).unwrap()["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    // A blob no session or snapshot names.
    let orphan = blob_store()
        .put(b"nothing refers to this", Default::default())
        .await
        .unwrap();
    assert!(blob_store().has(&orphan).await.unwrap());

    let (status, body) = call(&app, "POST", "/v1/gc", &key, None).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    // The collection runs in the background, so wait for its effect.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while blob_store().has(&orphan).await.unwrap() {
        assert!(
            std::time::Instant::now() < deadline,
            "the orphan was not swept"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // The live session still has its file.
    let (status, bytes) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/keep.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"kept bytes".to_vec());

    // The snapshot still restores its file after the collection.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{snapshot_id}/fork"),
        &key,
        json_body(&json!({ "count": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let forked = serde_json::from_slice::<Value>(&body).unwrap()["session_ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    let (status, bytes) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{forked}/fs/keep.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"kept bytes".to_vec());

    // A second collection inside the interval is refused.
    let (status, body) = call(&app, "POST", "/v1/gc", &key, None).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn the_schedule_sweeps_without_a_request() {
    let _serial = SERIAL.lock().await;
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, state) = create_router_with_state(Arc::new(Scheduler::new()), auth);
    let id = create(&app, &key).await;
    exec(&app, &key, &id, &["echo", "scheduled"]).await;
    let orphan = blob_store()
        .put(b"scheduled orphan", Default::default())
        .await
        .unwrap();

    let schedule = sandbox_server::gc::schedule(state, std::time::Duration::from_millis(50));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while blob_store().has(&orphan).await.unwrap() {
        assert!(
            std::time::Instant::now() < deadline,
            "the schedule did not sweep"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    schedule.abort();
}
