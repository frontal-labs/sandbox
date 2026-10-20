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

#[tokio::test]
async fn session_limit_is_per_key() {
    use sandbox_server::api::MAX_SESSIONS_PER_KEY;
    let (app, owner, other) = router();
    for _ in 0..MAX_SESSIONS_PER_KEY {
        create(&app, &owner).await;
    }
    let (status, body) = call(&app, "POST", "/v1/sessions", &owner, json_body(&config())).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "RESOURCE_EXHAUSTED");

    // One key's sessions must not stop another key from creating its own.
    create(&app, &other).await;
}

#[tokio::test]
async fn snapshot_limit_is_per_key() {
    use sandbox_server::api::MAX_SNAPSHOTS_PER_KEY;
    let (app, owner, other) = router();
    let id = create(&app, &owner).await;
    for _ in 0..MAX_SNAPSHOTS_PER_KEY {
        snapshot(&app, &owner, &id).await;
    }
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let other_id = create(&app, &other).await;
    snapshot(&app, &other, &other_id).await;
}

async fn get_json(app: &axum::Router, key: &str, uri: &str) -> (StatusCode, Value) {
    let (status, body) = call(app, "GET", uri, key, None).await;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn snapshot_listing_is_per_session_and_per_key() {
    let (app, owner, other) = router();
    let first = create(&app, &owner).await;
    let second = create(&app, &owner).await;
    let first_snap = snapshot(&app, &owner, &first).await;
    let second_snap = snapshot(&app, &owner, &second).await;

    let (status, list) = get_json(&app, &owner, &format!("/v1/sessions/{first}/snapshots")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![first_snap.as_str()],
        "only this session's snapshots"
    );
    assert_ne!(first_snap, second_snap);

    // Another key cannot list the session at all.
    let (status, _) = get_json(&app, &other, &format!("/v1/sessions/{first}/snapshots")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_snapshot_returns_it_only_under_its_own_session() {
    let (app, owner, other) = router();
    let first = create(&app, &owner).await;
    let second = create(&app, &owner).await;
    let snap = snapshot(&app, &owner, &first).await;

    let (status, body) = get_json(
        &app,
        &owner,
        &format!("/v1/sessions/{first}/snapshots/{snap}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], snap);
    assert_eq!(body["tier"], "isolate");
    assert!(body["vfs_root"].as_str().unwrap().len() == 64);

    // The same snapshot under a different session is not found.
    let (status, _) = get_json(
        &app,
        &owner,
        &format!("/v1/sessions/{second}/snapshots/{snap}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Another key is not found either.
    let (status, _) = get_json(
        &app,
        &other,
        &format!("/v1/sessions/{first}/snapshots/{snap}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn empty_limits_mean_defaults_and_are_accepted() {
    let (app, key, _) = router();
    let mut with_empty_limits: Value = config();
    with_empty_limits["limits"] = json!({});
    let (status, body) = call(
        &app,
        "POST",
        "/v1/sessions",
        &key,
        json_body(&with_empty_limits),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

/// Splits an NDJSON body into its parsed lines.
fn ndjson(body: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(body)
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect()
}

#[tokio::test]
async fn streamed_exec_sends_output_then_one_exit_line() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let request = json!({ "argv": ["echo", "hi"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": true });
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        json_body(&request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let lines = ndjson(&body);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["type"], "output");
    assert_eq!(lines[0]["stream"], "stdout");
    let text: Vec<u8> = lines[0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(text, b"hi\n");
    assert_eq!(lines[1]["type"], "exit");
    assert_eq!(lines[1]["exit_code"], 0);
}

#[tokio::test]
async fn query_flag_streams_the_same_way() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec?stream=true"),
        &key,
        json_body(&json!({ "argv": ["echo", "hi"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(ndjson(&body).last().unwrap()["type"], "exit");
}

#[tokio::test]
async fn a_streamed_command_cut_short_ends_with_an_error_line() {
    // The status is already 200 when the timeout fires, so the failure is a line.
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let request = json!({
        "argv": ["node", "-e", "let i=0; while(i<1000000000){i++}"],
        "env": {}, "cwd": null, "stdin": null, "timeout_ms": 50, "stream": true
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
    let last = ndjson(&body).pop().expect("at least one line");
    assert_eq!(last["type"], "error");
    assert_eq!(last["code"], "TIMEOUT");
}

#[tokio::test]
async fn streamed_exec_still_refuses_bad_requests_before_streaming() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec?stream=true"),
        &key,
        json_body(&json!({ "argv": [], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV/exec?stream=true",
        &key,
        json_body(&json!({ "argv": ["echo", "hi"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn timestamps_are_rfc3339_strings_on_the_wire() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, body) = get_json(&app, &key, &format!("/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    let created = body["created_at"].as_str().expect("created_at is a string");
    assert!(created.ends_with('Z') && created.contains('T'), "{created}");
}

async fn sleep(app: &axum::Router, key: &str, id: &str) -> (StatusCode, Value) {
    let (status, body) = call(app, "POST", &format!("/v1/sessions/{id}/sleep"), key, None).await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn wake(app: &axum::Router, key: &str, id: &str) -> (StatusCode, Value) {
    let (status, body) = call(app, "POST", &format!("/v1/sessions/{id}/wake"), key, None).await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn sleep_releases_the_session_and_wake_restores_its_files() {
    let (app, owner, _) = router();
    let id = create(&app, &owner).await;
    write_file(&app, &owner, &id, "/keep.txt", b"kept").await;

    let (status, body) = sleep(&app, &owner, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["snapshot_id"].is_string());

    let (status, state) = get_json(&app, &owner, &format!("/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["status"], "sleeping");

    // A request to a sleeping session wakes it first (PLAN §4.14). The command
    // runs on the restored files.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &owner,
        json_body(&json!({
            "argv": ["cat", "/keep.txt"],
            "env": {},
            "cwd": null,
            "stdin": null,
            "timeout_ms": null,
            "stream": false,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, state) = get_json(&app, &owner, &format!("/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["status"], "running");

    // Sleeping again, a snapshot request wakes it too.
    assert_eq!(sleep(&app, &owner, &id).await.0, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, bytes) = read_file(&app, &owner, &id, "/keep.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"kept");
    write_file(&app, &owner, &id, "/after.txt", b"again").await;
}

#[tokio::test]
async fn sleep_snapshot_can_be_forked_into_a_new_session() {
    let (app, owner, _) = router();
    let id = create(&app, &owner).await;
    write_file(&app, &owner, &id, "/f.txt", b"slept").await;
    let (_, body) = sleep(&app, &owner, &id).await;
    let snapshot_id = body["snapshot_id"].as_str().unwrap().to_string();

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{snapshot_id}/fork"),
        &owner,
        json_body(&json!({"count": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let forked: Value = serde_json::from_slice(&body).unwrap();
    let new_id = forked["session_ids"][0].as_str().unwrap().to_string();
    let (status, bytes) = read_file(&app, &owner, &new_id, "/f.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"slept");
}

#[tokio::test]
async fn sleep_and_wake_are_conflicts_in_the_wrong_state() {
    let (app, owner, _) = router();
    let id = create(&app, &owner).await;

    let (status, _) = wake(&app, &owner, &id).await;
    assert_eq!(status, StatusCode::CONFLICT, "waking a running session");

    assert_eq!(sleep(&app, &owner, &id).await.0, StatusCode::OK);
    assert_eq!(
        sleep(&app, &owner, &id).await.0,
        StatusCode::CONFLICT,
        "sleeping twice"
    );
    assert_eq!(wake(&app, &owner, &id).await.0, StatusCode::OK);
    assert_eq!(
        wake(&app, &owner, &id).await.0,
        StatusCode::CONFLICT,
        "waking twice"
    );
}

#[tokio::test]
async fn sleeping_sessions_are_invisible_to_other_keys() {
    let (app, owner, other) = router();
    let id = create(&app, &owner).await;
    assert_eq!(sleep(&app, &other, &id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(sleep(&app, &owner, &id).await.0, StatusCode::OK);
    assert_eq!(wake(&app, &other, &id).await.0, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "DELETE", &format!("/v1/sessions/{id}"), &other, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_sleeping_session_can_be_destroyed_without_waking() {
    let (app, owner, _) = router();
    let id = create(&app, &owner).await;
    assert_eq!(sleep(&app, &owner, &id).await.0, StatusCode::OK);
    let (status, _) = call(&app, "DELETE", &format!("/v1/sessions/{id}"), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = get_json(&app, &owner, &format!("/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn exec_argv(app: &axum::Router, key: &str, id: &str, argv: &[&str]) -> (StatusCode, Value) {
    let request = json!({
        "argv": argv,
        "env": {},
        "cwd": null,
        "stdin": null,
        "timeout_ms": null,
        "stream": false,
    });
    let (status, body) = call(
        app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        key,
        json_body(&request),
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_trace_records_commands_and_replays_to_the_same_outputs() {
    use sandbox_session::IsolateBackend;
    use sandbox_trace::{replay, Trace};

    let (app, key, _) = router();
    let id = create(&app, &key).await;
    // Arguments with quotes and spaces must keep their meaning in the trace.
    let (status, _) = exec_argv(&app, &key, &id, &["echo", "it's", "a  test"]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = exec_argv(&app, &key, &id, &["echo", "second"]).await;
    assert_eq!(status, StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/sessions/{id}/trace"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/x-ndjson");
    assert_eq!(response.headers()["x-trace-complete"], "true");
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    let trace = Trace::parse(&text).unwrap();
    assert_eq!(trace.runs.len(), 2);

    // Replaying the trace in a fresh sandbox gives the recorded outputs.
    let session = IsolateBackend::new(sandbox_types::SessionConfig::default())
        .await
        .unwrap();
    let report = replay(&trace, |run| {
        let session = &session;
        let run = run.clone();
        async move { sandbox_session::replay_run(session, &run).await }
    })
    .await;
    assert!(report.matches(), "{:?}", report.divergences);
    assert_eq!(report.runs, 2);
}

#[tokio::test]
async fn the_trace_lists_the_files_each_command_changed() {
    use sandbox_trace::{FileChange, Trace};
    use sandbox_types::event::FileOp;

    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, _) = exec_argv(&app, &key, &id, &["mkdir", "/made"]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = exec_argv(&app, &key, &id, &["touch", "/made/a.txt"]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = exec_argv(&app, &key, &id, &["cat", "/made/a.txt"]).await;
    assert_eq!(status, StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/sessions/{id}/trace"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["x-trace-complete"], "true");
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    let trace = Trace::parse(&text).unwrap();
    assert_eq!(trace.runs.len(), 3);
    assert_eq!(
        trace.runs[0].files,
        vec![FileChange {
            path: "/made".to_string(),
            op: FileOp::Mkdir,
            before: None,
            after: None,
        }]
    );
    assert_eq!(trace.runs[1].files.len(), 1);
    assert_eq!(trace.runs[1].files[0].path, "/made/a.txt");
    assert_eq!(trace.runs[1].files[0].op, FileOp::Create);
    // A read changes nothing, so the run lists no files.
    assert!(trace.runs[2].files.is_empty());
}

#[tokio::test]
async fn an_exec_past_its_timeout_returns_504_with_the_timeout_code() {
    let (app, key, _) = router();
    let id = create(&app, &key).await;
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        json_body(&json!({
            "argv": ["node", "-e", "let i=0; while(i<1000000000){i++}"],
            "env": {}, "cwd": null, "stdin": null, "timeout_ms": 50, "stream": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "TIMEOUT");
}

#[tokio::test]
async fn filesystem_writes_and_deletes_are_traced_and_replay_to_the_same_state() {
    use sandbox_session::{replay_run, IsolateBackend, Session};
    use sandbox_trace::{replay, FsOp, Trace};
    use sandbox_types::event::FileOp;

    let (app, key, _) = router();
    let id = create(&app, &key).await;
    // The write API does not create parent directories, so make one first.
    let (status, _) = exec_argv(&app, &key, &id, &["mkdir", "/notes"]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/notes/a.txt"),
        &key,
        Some(("application/octet-stream", b"hello file".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/notes/b.txt"),
        &key,
        Some(("application/octet-stream", b"second".to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/sessions/{id}/fs/notes/a.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = exec_argv(&app, &key, &id, &["cat", "/notes/b.txt"]).await;
    assert_eq!(status, StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/sessions/{id}/trace"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["x-trace-complete"], "true");
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let trace = Trace::parse(&text).unwrap();
    assert_eq!(trace.runs.len(), 5);

    // Scripts have no filesystem operation, and filesystem runs have no script.
    assert!(trace.runs[0].fs.is_none());
    assert!(trace.runs[4].fs.is_none());
    assert_eq!(
        trace.runs[1].fs,
        Some(FsOp::Write {
            path: "/notes/a.txt".to_string(),
            data: b"hello file".to_vec(),
        })
    );
    assert_eq!(trace.runs[1].script, "");
    let created: Vec<(&str, FileOp)> = trace.runs[1]
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.op))
        .collect();
    assert_eq!(created, vec![("/notes/a.txt", FileOp::Create)]);
    assert_eq!(
        trace.runs[3].fs,
        Some(FsOp::Delete {
            path: "/notes/a.txt".to_string(),
        })
    );
    let removed: Vec<(&str, FileOp)> = trace.runs[3]
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.op))
        .collect();
    assert_eq!(removed, vec![("/notes/a.txt", FileOp::Delete)]);

    // Replaying in a fresh sandbox makes the same writes, so every outcome matches.
    let session = IsolateBackend::new(sandbox_types::SessionConfig::default())
        .await
        .unwrap();
    let report = replay(&trace, |run| {
        let session = &session;
        let run = run.clone();
        async move { replay_run(session, &run).await }
    })
    .await;
    assert!(report.matches(), "{:?}", report.divergences);
    assert_eq!(report.runs, 5);
    // The replayed session holds b.txt and not a.txt, as the original did.
    let out = session.run_script("cat /notes/b.txt").await.unwrap();
    assert_eq!(out.stdout, b"second".to_vec());
    assert_ne!(
        session
            .run_script("cat /notes/a.txt")
            .await
            .unwrap()
            .exit_code,
        0
    );
}

#[tokio::test]
async fn a_trace_is_readable_only_by_the_session_owner() {
    let (app, owner, other) = router();
    let id = create(&app, &owner).await;
    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/trace"),
        &other,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_trace_stops_recording_at_its_byte_budget_and_says_so() {
    use sandbox_server::api::MAX_TRACE_BYTES;

    let (app, key, _) = router();
    let id = create(&app, &key).await;
    // A 1 MiB file, read back nine times: nine outputs of 1 MiB exceed the budget.
    let big = vec![b'x'; 1024 * 1024];
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/v1/sessions/{id}/fs/big.txt"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(big))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    for _ in 0..9 {
        let (status, _) = exec_argv(&app, &key, &id, &["cat", "/big.txt"]).await;
        assert_eq!(status, StatusCode::OK);
    }

    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/sessions/{id}/trace"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["x-trace-complete"], "false");
    let text = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let recorded = text.lines().count() - 1;
    assert!(
        recorded < 9,
        "the budget should stop recording: {recorded} runs"
    );
    assert!(recorded * 1024 * 1024 <= MAX_TRACE_BYTES);
}
