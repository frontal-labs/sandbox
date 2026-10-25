//! Per-session filesystem quotas (ADR 0014), driven through the real router.
//!
//! The quotas are the VFS limits: file size, total size, inode count, and path
//! depth. Each test checks that a limit still holds after the session takes a
//! path that could drop it: a sleep and wake, or a fork of a snapshot.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

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

/// A session config with the given limits set, and everything else default.
fn config_with(limits: Value) -> Value {
    let mut config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    config["limits"] = limits;
    config
}

fn router() -> (axum::Router, String) {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    (create_router(Arc::new(Scheduler::new()), auth), key)
}

async fn create_with(app: &axum::Router, key: &str, limits: Value) -> String {
    let (status, body) = call(
        app,
        "POST",
        "/v1/sessions",
        key,
        json_body(&config_with(limits)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["session_id"].as_str().unwrap().to_string()
}

async fn put(app: &axum::Router, key: &str, id: &str, path: &str, bytes: &[u8]) -> StatusCode {
    call(
        app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/{path}"),
        key,
        Some(("application/octet-stream", bytes.to_vec())),
    )
    .await
    .0
}

/// Limits that set only the file size, and leave the rest to the defaults.
fn max_file_size(bytes: u64) -> Value {
    json!({ "max_file_size": bytes })
}

#[tokio::test]
async fn a_file_size_quota_is_enforced_on_writes() {
    let (app, key) = router();
    let id = create_with(&app, &key, max_file_size(8)).await;
    assert_eq!(
        put(&app, &key, &id, "small.txt", b"1234").await,
        StatusCode::NO_CONTENT
    );
    // Over the quota, the write is refused as a resource limit.
    assert_eq!(
        put(&app, &key, &id, "big.txt", b"123456789").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_quota_survives_sleep_and_wake() {
    let (app, key) = router();
    let id = create_with(&app, &key, max_file_size(8)).await;
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/sleep"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, body) = call(&app, "POST", &format!("/v1/sessions/{id}/wake"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        put(&app, &key, &id, "big.txt", b"123456789").await,
        StatusCode::TOO_MANY_REQUESTS,
        "a woken session must keep its quota"
    );
}

#[tokio::test]
async fn a_fork_keeps_the_quota_of_the_session_it_came_from() {
    let (app, key) = router();
    let id = create_with(&app, &key, max_file_size(8)).await;
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
    assert_eq!(
        put(&app, &key, &forked, "big.txt", b"123456789").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// The bytes of the environment file a session starts with (IMAGE §4.5). Every
/// session has the same length, because its one variable field is a fixed-width ID.
async fn environment_file_len(app: &axum::Router, key: &str) -> usize {
    let id = create_with(app, key, json!({})).await;
    let (status, body) = call(
        app,
        "GET",
        &format!("/v1/sessions/{id}/fs/etc/agent/env.json"),
        key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body.len()
}

#[tokio::test]
async fn a_total_size_quota_counts_every_file() {
    let (app, key) = router();
    // The environment file counts too, so the quota leaves room for it and 10 bytes more.
    let quota = environment_file_len(&app, &key).await as u64 + 10;
    let id = create_with(&app, &key, json!({ "max_total_size": quota })).await;
    assert_eq!(
        put(&app, &key, &id, "a.txt", b"123456").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(&app, &key, &id, "b.txt", b"123456").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn the_environment_file_counts_against_a_quota_smaller_than_it() {
    // A quota below the file's size refuses writes. The session still starts,
    // because the file is written before quotas apply.
    let (app, key) = router();
    let id = create_with(&app, &key, json!({ "max_total_size": 10 })).await;
    assert_eq!(
        put(&app, &key, &id, "a.txt", b"1").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_zero_quota_is_refused_as_an_invalid_argument() {
    let (app, key) = router();
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sessions",
        &key,
        json_body(&config_with(max_file_size(0))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn limits_that_are_not_enforced_are_still_refused() {
    let (app, key) = router();
    for limits in [
        json!({ "max_processes": 5 }),
        json!({ "max_open_files": 5 }),
        json!({ "cpu_ms_per_sec": 5 }),
    ] {
        let (status, _) = call(
            &app,
            "POST",
            "/v1/sessions",
            &key,
            json_body(&config_with(limits.clone())),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{limits}");
    }
}
