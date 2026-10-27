//! Checkpoints, undo, and fork-with-command in Tier 1 (IMAGE.md §4.5), driven through
//! the real router.

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

fn json_call(value: &Value) -> Option<(&'static str, Vec<u8>)> {
    Some(("application/json", serde_json::to_vec(value).unwrap()))
}

fn setup() -> (axum::Router, String) {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    (create_router(Arc::new(Scheduler::new()), auth), key)
}

/// A Tier 1 session config, with the checkpoint settings merged in.
fn config_with(extra: Value) -> Value {
    let mut config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    for (key, value) in extra.as_object().unwrap() {
        config[key] = value.clone();
    }
    config
}

async fn create(app: &axum::Router, key: &str, extra: Value) -> String {
    let (status, body) = call(
        app,
        "POST",
        "/v1/sessions",
        key,
        json_call(&config_with(extra)),
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

async fn put(app: &axum::Router, key: &str, id: &str, path: &str, text: &str) {
    let (status, body) = call(
        app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/{path}"),
        key,
        Some(("application/octet-stream", text.as_bytes().to_vec())),
    )
    .await;
    assert!(status.is_success(), "{}", String::from_utf8_lossy(&body));
}

/// The file's text, or `None` when it does not exist.
async fn get(app: &axum::Router, key: &str, id: &str, path: &str) -> Option<String> {
    let (status, body) = call(
        app,
        "GET",
        &format!("/v1/sessions/{id}/fs/{path}"),
        key,
        None,
    )
    .await;
    match status {
        StatusCode::OK => Some(String::from_utf8(body).unwrap()),
        StatusCode::NOT_FOUND => None,
        other => panic!(
            "unexpected status {other}: {}",
            String::from_utf8_lossy(&body)
        ),
    }
}

async fn exec_ok(app: &axum::Router, key: &str, id: &str, argv: &[&str]) -> Value {
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

async fn checkpoint_count(app: &axum::Router, key: &str, id: &str) -> u64 {
    let (status, body) = call(app, "GET", &format!("/v1/sessions/{id}"), key, None).await;
    assert_eq!(status, StatusCode::OK);
    json_of(&body)["checkpoints"]["count"].as_u64().unwrap()
}

async fn undo(app: &axum::Router, key: &str, id: &str, steps: u64) -> (StatusCode, Vec<u8>) {
    call(
        app,
        "POST",
        &format!("/v1/sessions/{id}/undo"),
        key,
        json_call(&json!({ "steps": steps })),
    )
    .await
}

#[tokio::test]
async fn each_exec_pins_a_checkpoint_and_undo_steps_back_through_them() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({})).await;
    assert_eq!(
        checkpoint_count(&app, &key, &id).await,
        1,
        "the initial state is pinned"
    );

    put(&app, &key, &id, "a.txt", "one").await;
    exec_ok(&app, &key, &id, &["true"]).await;
    put(&app, &key, &id, "a.txt", "two").await;
    exec_ok(&app, &key, &id, &["true"]).await;
    assert_eq!(checkpoint_count(&app, &key, &id).await, 3);

    let (status, body) = undo(&app, &key, &id, 1).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(get(&app, &key, &id, "a.txt").await.as_deref(), Some("one"));

    let (status, _) = undo(&app, &key, &id, 1).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        get(&app, &key, &id, "a.txt").await,
        None,
        "back to the empty initial state"
    );

    let (status, body) = undo(&app, &key, &id, 1).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn retention_unpins_the_oldest_checkpoints() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({"checkpoint_retain": 2})).await;
    for text in ["one", "two", "three"] {
        put(&app, &key, &id, "a.txt", text).await;
        exec_ok(&app, &key, &id, &["true"]).await;
    }
    assert_eq!(
        checkpoint_count(&app, &key, &id).await,
        2,
        "only the last two are pinned"
    );

    let (status, _) = undo(&app, &key, &id, 2).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the older checkpoints are gone"
    );
    let (status, _) = undo(&app, &key, &id, 1).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(get(&app, &key, &id, "a.txt").await.as_deref(), Some("two"));
}

#[tokio::test]
async fn a_manual_session_pins_only_when_asked() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({"checkpoint": "manual"})).await;
    put(&app, &key, &id, "a.txt", "one").await;
    exec_ok(&app, &key, &id, &["true"]).await;
    assert_eq!(
        checkpoint_count(&app, &key, &id).await,
        1,
        "exec does not pin when manual"
    );

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/checkpoint"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(checkpoint_count(&app, &key, &id).await, 2);
}

#[tokio::test]
async fn get_reports_the_checkpoint_bytes_and_policy() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({})).await;
    put(&app, &key, &id, "a.txt", "12345").await;
    exec_ok(&app, &key, &id, &["true"]).await;

    let (_, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    let view = json_of(&body);
    assert_eq!(view["id"], id, "the state fields stay at the top level");
    assert_eq!(view["checkpoints"]["policy"], "per_exec");
    assert_eq!(view["checkpoints"]["retain"], 50);
    assert_eq!(view["checkpoints"]["count"], 2);
    // The initial root holds nothing. The root after the exec holds the 5-byte file.
    assert_eq!(view["checkpoints"]["bytes"], 5);
}

#[tokio::test]
async fn info_returns_the_environment() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({})).await;
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}/info"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let env = json_of(&body);
    assert_eq!(env["schema"], 1);
    assert_eq!(env["session"], id);
}

#[tokio::test]
async fn fork_runs_the_command_in_independent_children() {
    let (app, key) = setup();
    let parent = create(&app, &key, json!({})).await;
    put(&app, &key, &parent, "base.txt", "shared").await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{parent}/fork"),
        &key,
        json_call(&json!({"count": 3, "argv": ["touch", "/r"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let children = json_of(&body)["children"].as_array().unwrap().clone();
    assert_eq!(children.len(), 3);
    let ids: Vec<String> = children
        .iter()
        .map(|c| c["session_id"].as_str().unwrap().to_string())
        .collect();
    for child in &children {
        assert_eq!(child["exit_code"], 0, "{child}");
        assert!(child["changed_files"].as_u64().unwrap() >= 1, "{child}");
    }
    assert!(!ids.contains(&parent));

    for id in &ids {
        assert_eq!(
            get(&app, &key, id, "base.txt").await.as_deref(),
            Some("shared")
        );
    }
    // A write in one child does not reach its siblings.
    put(&app, &key, &ids[0], "mine.txt", "only here").await;
    assert_eq!(
        get(&app, &key, &ids[0], "mine.txt").await.as_deref(),
        Some("only here")
    );
    assert_eq!(get(&app, &key, &ids[1], "mine.txt").await, None);
    assert_eq!(get(&app, &key, &parent, "mine.txt").await, None);
}

#[tokio::test]
async fn fork_refuses_an_empty_command_and_a_zero_count() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({})).await;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/fork"),
        &key,
        json_call(&json!({"count": 1, "argv": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/fork"),
        &key,
        json_call(&json!({"count": 0, "argv": ["true"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

fn stdout_text(result: &Value) -> String {
    let bytes: Vec<u8> = result["stdout"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn sandbox_commands_typed_inside_the_session_run_the_host_operations() {
    let (app, key) = setup();
    let id = create(&app, &key, json!({})).await;
    put(&app, &key, &id, "a.txt", "one").await;
    exec_ok(&app, &key, &id, &["true"]).await;
    put(&app, &key, &id, "a.txt", "two").await;

    let pinned = exec_ok(&app, &key, &id, &["sandbox", "checkpoint"]).await;
    assert_eq!(pinned["exit_code"], 0, "{pinned}");
    assert_eq!(
        stdout_text(&pinned).trim().len(),
        64,
        "prints the root hash"
    );

    let undone = exec_ok(&app, &key, &id, &["sandbox", "undo", "1"]).await;
    assert_eq!(undone["exit_code"], 0, "{undone}");
    assert_eq!(get(&app, &key, &id, "a.txt").await.as_deref(), Some("one"));

    let forked = exec_ok(
        &app,
        &key,
        &id,
        &["sandbox", "fork", "2", "--", "touch", "/r"],
    )
    .await;
    assert_eq!(forked["exit_code"], 0, "{forked}");
    let lines: Vec<String> = stdout_text(&forked).lines().map(String::from).collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines.iter().all(|line| line.contains(" ok changed=")),
        "{lines:?}"
    );
    let child = lines[0].split(' ').next().unwrap();
    assert_eq!(
        get(&app, &key, child, "a.txt").await.as_deref(),
        Some("one")
    );

    let bad = exec_ok(&app, &key, &id, &["sandbox", "fork", "2"]).await;
    assert_eq!(
        bad["exit_code"], 2,
        "a fork without a command is a usage error"
    );
}
