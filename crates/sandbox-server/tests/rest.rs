//! End-to-end checks of the session, exec, filesystem, and snapshot routes,
//! driven through the real router.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::policy::{NetworkPolicy, ResourceLimits};
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::collections::HashMap;
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

fn config() -> Value {
    serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        limits: ResourceLimits::default(),
        network: NetworkPolicy::default(),
        env: HashMap::new(),
        cwd: "/".to_string(),
        initial_snapshot: None,
        secrets: None,
        labels: HashMap::new(),
    })
    .unwrap()
}

fn router() -> (axum::Router, String, String) {
    let auth = Arc::new(AuthManager::new());
    let owner = auth.create_key();
    let other = auth.create_key();
    (
        create_router(Arc::new(Scheduler::new()), auth),
        owner,
        other,
    )
}

async fn create(app: &axum::Router, key: &str) -> String {
    let (status, body) = call(app, "POST", "/v1/sessions", key, json_body(&config())).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["session_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn session_lifecycle_round_trip() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;

    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::OK);
    let state: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(state["id"], id);

    let (status, _) = call(&app, "DELETE", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "destroyed session must be gone"
    );
}

#[tokio::test]
async fn exec_runs_a_command_in_the_session() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let request = json!({
        "argv": ["echo", "hello"],
        "env": {},
        "cwd": null,
        "stdin": null,
        "timeout_ms": null,
        "stream": false,
    });
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        json_body(&request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let result: Value = serde_json::from_slice(&body).unwrap();
    let stdout: Vec<u8> = result["stdout"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(String::from_utf8_lossy(&stdout).trim(), "hello");
}

#[tokio::test]
async fn file_write_then_read_returns_the_same_bytes() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let uri = format!("/v1/sessions/{id}/fs/notes.txt");

    let (status, _) = call(
        &app,
        "PUT",
        &uri,
        &key,
        Some(("application/octet-stream", b"hello file".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call(&app, "GET", &uri, &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"hello file");

    let (status, _) = call(&app, "DELETE", &uri, &key, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, "GET", &uri, &key, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sessions_are_invisible_to_other_keys() {
    let (app, owner, other) = router();
    let id = create(&app, &owner).await;

    let (status, _) = call(&app, "GET", &format!("/v1/sessions/{id}"), &other, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another key must not see the session"
    );

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/x.txt"),
        &other,
        Some(("application/octet-stream", b"nope".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another key must not write");

    let (status, _) = call(&app, "DELETE", &format!("/v1/sessions/{id}"), &other, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another key must not destroy"
    );

    // The owner still has it.
    let (status, _) = call(&app, "GET", &format!("/v1/sessions/{id}"), &owner, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn paths_cannot_climb_out_of_the_session() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/a/../../etc/passwd"),
        &key,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn unknown_session_is_not_found() {
    let (app, key, _) = router();
    let (status, _) = call(
        &app,
        "GET",
        "/v1/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV",
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn snapshot_and_fork_are_honestly_unsupported() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "UNSUPPORTED");

    let (status, body) = call(
        &app,
        "POST",
        "/v1/snapshots/01ARZ3NDEKTSV4RRFFQ69G5FAV/fork",
        &key,
        json_body(&json!({ "count": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "UNSUPPORTED");
}
