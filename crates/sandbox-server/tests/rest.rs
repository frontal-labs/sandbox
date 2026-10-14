//! End-to-end checks of the session, exec, filesystem, and snapshot routes,
//! driven through the real router.

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

fn config() -> Value {
    serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
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

async fn write_file(app: &axum::Router, key: &str, id: &str, path: &str, bytes: &[u8]) {
    let (status, body) = call(
        app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/{path}"),
        key,
        Some(("application/octet-stream", bytes.to_vec())),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

async fn read_file(app: &axum::Router, key: &str, id: &str, path: &str) -> (StatusCode, Vec<u8>) {
    call(
        app,
        "GET",
        &format!("/v1/sessions/{id}/fs/{path}"),
        key,
        None,
    )
    .await
}

async fn snapshot(app: &axum::Router, key: &str, id: &str) -> String {
    let (status, body) = call(
        app,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        key,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["snapshot_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn snapshot_and_fork_restore_the_committed_files() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    write_file(&app, &key, &id, "data.txt", b"saved").await;
    let snapshot_id = snapshot(&app, &key, &id).await;

    // Writes after the snapshot must not appear in the fork.
    write_file(&app, &key, &id, "data.txt", b"changed").await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{snapshot_id}/fork"),
        &key,
        json_body(&json!({ "count": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let forked: Value = serde_json::from_slice(&body).unwrap();
    let ids = forked["session_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    for session in ids {
        let session = session.as_str().unwrap();
        let (status, bytes) = read_file(&app, &key, session, "data.txt").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"saved");
    }
}

#[tokio::test]
async fn initial_snapshot_starts_a_session_from_the_snapshot() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    write_file(&app, &key, &id, "seed.txt", b"seed").await;
    let snapshot_id = snapshot(&app, &key, &id).await;

    let mut config: Value = config();
    config["initial_snapshot"] = json!(snapshot_id);
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, json_body(&config)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: Value = serde_json::from_slice(&body).unwrap();
    let new_id = created["session_id"].as_str().unwrap();
    let (status, bytes) = read_file(&app, &key, new_id, "seed.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"seed");
}

#[tokio::test]
async fn snapshots_are_invisible_to_other_keys() {
    let (app, owner, other) = router();
    let id = create(&app, &owner).await;
    let snapshot_id = snapshot(&app, &owner, &id).await;

    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{snapshot_id}/fork"),
        &other,
        json_body(&json!({ "count": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another key must not fork");

    let mut config: Value = config();
    config["initial_snapshot"] = json!(snapshot_id);
    let (status, _) = call(&app, "POST", "/v1/sessions", &other, json_body(&config)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another key must not start from it"
    );
}

#[tokio::test]
async fn fork_count_is_bounded() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let snapshot_id = snapshot(&app, &key, &id).await;
    for count in [0u32, 65] {
        let (status, _) = call(
            &app,
            "POST",
            &format!("/v1/snapshots/{snapshot_id}/fork"),
            &key,
            json_body(&json!({ "count": count })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "count {count}");
    }
}

#[tokio::test]
async fn settings_the_backend_does_not_enforce_are_refused_not_ignored() {
    let (app, key, _) = router();
    let mut with_secrets: Value = config();
    with_secrets["secrets"] =
        json!({ "secrets": { "TOKEN": { "value": "x", "hosts": ["example.com"] } } });
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, json_body(&with_secrets)).await;
    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "UNSUPPORTED");

    let mut with_limits: Value = config();
    with_limits["limits"] = json!({ "memory_bytes": 1024, "cpu_ms_per_sec": null, "wall_timeout_secs": null, "max_file_size": null, "max_total_size": null, "max_inodes": null, "max_path_depth": null, "max_processes": null, "max_open_files": null });
    let (status, _) = call(&app, "POST", "/v1/sessions", &key, json_body(&with_limits)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}
