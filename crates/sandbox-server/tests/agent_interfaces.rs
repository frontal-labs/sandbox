//! Agent-facing interfaces in Tier 1 (IMAGE.md §4.5), driven through the real router:
//! `/etc/agent/env.json`, the quota exemption it needs, output capping, and the
//! full-output path that a capped marker names.

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

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

fn json_call(body: &Value) -> Option<(&'static str, Vec<u8>)> {
    Some(("application/json", serde_json::to_vec(body).unwrap()))
}

fn setup() -> (axum::Router, String) {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    (create_router(Arc::new(Scheduler::new()), auth), key)
}

/// A Tier 1 session config. `limits` is merged in when given.
fn config_with(limits: Option<Value>) -> Value {
    let mut config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    if let Some(limits) = limits {
        config["limits"] = limits;
    }
    config
}

async fn create(app: &axum::Router, key: &str, limits: Option<Value>) -> String {
    let (status, body) = call(
        app,
        "POST",
        "/v1/sessions",
        key,
        json_call(&config_with(limits)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    json_of(&body)["session_id"].as_str().unwrap().to_string()
}

async fn read_env_json(app: &axum::Router, key: &str, id: &str) -> Value {
    let (status, body) = call(
        app,
        "GET",
        &format!("/v1/sessions/{id}/fs/etc/agent/env.json"),
        key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    json_of(&body)
}

async fn exec(app: &axum::Router, key: &str, id: &str, argv: &[&str]) -> Value {
    let request = json!({"argv": argv, "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false});
    let (status, body) = call(
        app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        key,
        json_call(&request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    json_of(&body)
}

#[tokio::test]
async fn a_new_tier1_session_has_a_schema_1_env_json() {
    let (app, key) = setup();
    let id = create(&app, &key, None).await;

    let env = read_env_json(&app, &key, &id).await;
    assert_eq!(env["schema"], 1);
    assert_eq!(env["session"], id);
    assert_eq!(env["tier"], "isolate");
    assert_eq!(env["user"], "agent");
    assert_eq!(env["network"]["default"], "deny");
    assert_eq!(env["network"]["allow"], json!([]));
    // The default limits are 512 MiB of memory and a 300 s wall timeout.
    assert_eq!(env["limits"]["memory_mib"], 512);
    assert_eq!(env["limits"]["wall_timeout_s"], 300);
    assert_eq!(env["commands"], json!(["sandbox"]));
}

#[tokio::test]
async fn a_fork_gets_its_own_env_json_with_the_same_limits() {
    let (app, key) = setup();
    let parent = create(&app, &key, None).await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{parent}/snapshot"),
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
    let snapshot = json_of(&body)["snapshot_id"].as_str().unwrap().to_string();

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{snapshot}/fork"),
        &key,
        json_call(&json!({"count": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let child = json_of(&body)["session_ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(child, parent);

    let env = read_env_json(&app, &key, &child).await;
    assert_eq!(
        env["session"], child,
        "the fork names itself, not its parent"
    );
    assert_eq!(env["limits"]["wall_timeout_s"], 300);
    let parent_env = read_env_json(&app, &key, &parent).await;
    assert_eq!(parent_env["session"], parent);
}

#[tokio::test]
async fn env_json_does_not_use_up_the_session_quota() {
    // A total-size quota of 10 bytes is smaller than env.json, which must still
    // be written. The agent's own writes are the only bytes the quota counts.
    let (app, key) = setup();
    let id = create(&app, &key, Some(json!({"max_total_size": 10}))).await;
    read_env_json(&app, &key, &id).await;

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/a.txt"),
        &key,
        Some(("application/octet-stream", b"12345".to_vec())),
    )
    .await;
    assert!(
        status.is_success(),
        "a 5-byte write fits the 10-byte quota: {status}"
    );

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/b.txt"),
        &key,
        Some(("application/octet-stream", b"123456".to_vec())),
    )
    .await;
    assert!(!status.is_success(), "5 + 6 bytes exceed the 10-byte quota");
}

#[tokio::test]
async fn a_long_output_is_capped_and_its_full_stream_is_in_the_trace() {
    let (app, key) = setup();
    let id = create(&app, &key, None).await;

    let big: Vec<u8> = (0..20_000)
        .flat_map(|i| format!("line {i}\n").into_bytes())
        .collect();
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/big.txt"),
        &key,
        Some(("application/octet-stream", big.clone())),
    )
    .await;
    assert!(status.is_success(), "{}", String::from_utf8_lossy(&body));

    let result = exec(&app, &key, &id, &["cat", "/big.txt"]).await;
    let stdout: Vec<u8> = result["stdout"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    let text = String::from_utf8(stdout).unwrap();

    // Default cap: 8 KiB from each end, plus one marker line.
    assert!(text.len() < 17 * 1024, "capped to {} bytes", text.len());
    assert!(text.starts_with("line 0\n"));
    let marker_start = text.find("…[").expect("a marker line");
    let marker = &text[marker_start..text[marker_start..].find('\n').unwrap() + marker_start];
    assert!(
        marker.contains("lines omitted · full: /trace/exec/"),
        "{marker}"
    );
    let path = marker
        .split("full: ")
        .nth(1)
        .unwrap()
        .trim_end_matches(']')
        .to_string();

    // The marker's path returns the complete stream, byte for byte.
    let (status, full) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/{}", path.trim_start_matches('/')),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&full));
    assert_eq!(full, big);
}

#[tokio::test]
async fn output_within_the_cap_is_returned_unchanged() {
    let (app, key) = setup();
    let id = create(&app, &key, None).await;
    let result = exec(&app, &key, &id, &["echo", "hello"]).await;
    let stdout: Vec<u8> = result["stdout"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(stdout, b"hello\n");
}

#[tokio::test]
async fn an_unknown_trace_output_is_not_found() {
    let (app, key) = setup();
    let id = create(&app, &key, None).await;
    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/trace/exec/999.out"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_agent_cannot_change_or_remove_env_json() {
    // env.json is exempt from quotas, so the agent must not be able to write to
    // it, or remove it through its directory, at all.
    let (app, key) = setup();
    let id = create(&app, &key, None).await;
    let before = read_env_json(&app, &key, &id).await;

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/etc/agent/env.json"),
        &key,
        Some(("application/octet-stream", vec![b'x'; 1024 * 1024])),
    )
    .await;
    assert!(
        !status.is_success(),
        "a write to env.json must be refused: {status}"
    );

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/sessions/{id}/fs/etc/agent/env.json"),
        &key,
        None,
    )
    .await;
    assert!(
        !status.is_success(),
        "deleting env.json must be refused: {status}"
    );

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/sessions/{id}/fs/etc"),
        &key,
        None,
    )
    .await;
    assert!(
        !status.is_success(),
        "removing the directory above env.json must be refused: {status}"
    );

    assert_eq!(read_env_json(&app, &key, &id).await, before);
}
