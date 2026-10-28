//! A durable server keeps its keys, snapshots, and sleeping sessions across a
//! restart. These tests restart the server in one process. The blob store is
//! process-wide, so everything runs in one test, against one data directory.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use serde_json::{json, Value};
use std::path::PathBuf;
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

async fn call_bytes(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    bytes: &[u8],
) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, body)
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
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

fn exec(argv: &[&str]) -> Value {
    json!({"argv": argv, "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false})
}

#[tokio::test]
async fn keys_sleeping_sessions_and_snapshots_survive_a_restart() {
    let dir: PathBuf = std::env::temp_dir().join(format!("sandbox-durable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // First run: one key is created and returned, and nothing else exists yet.
    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");

    let (status, body) = call(&app, "POST", "/v1/sessions", &key, Some(config())).await;
    assert_eq!(status, StatusCode::CREATED);
    let sleeper = json_of(&body)["session_id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{sleeper}/exec"),
        &key,
        Some(exec(&["echo", "before"])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call_bytes(
        &app,
        "PUT",
        &format!("/v1/sessions/{sleeper}/fs/keep.txt"),
        &key,
        b"kept across the restart",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, Some(config())).await;
    assert_eq!(status, StatusCode::CREATED);
    let live = json_of(&body)["session_id"].as_str().unwrap().to_string();
    // The live session has a file, a shell variable, and a command in its history.
    let (status, body) = call_bytes(
        &app,
        "PUT",
        &format!("/v1/sessions/{live}/fs/live.txt"),
        &key,
        b"live bytes",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{live}/exec"),
        &key,
        Some(json!({"argv": ["echo", "live"], "env": {"GREETING": "hello"}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{sleeper}/sleep"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let sleep_snapshot = json_of(&body)["snapshot_id"].as_str().unwrap().to_string();

    // Restart: the old server is gone, and a new one reads the same directory.
    drop(app);
    let (app, none) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(none.is_none(), "a later run does not create a key");

    // The key from the first run still authenticates.
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{sleeper}"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json_of(&body)["status"], "sleeping");

    // The sleeping session still has its command trace.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{sleeper}/trace"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trace = String::from_utf8(body).unwrap();
    assert!(trace.contains("'echo' 'before'"), "{trace}");

    // The live session survives the restart, running, with its file and shell state.
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{live}"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json_of(&body)["status"], "running");
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{live}/fs/live.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"live bytes");
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{live}/exec"),
        &key,
        Some(json!({"argv": ["echo", "again"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json_of(&body)["stdout"].as_array().unwrap().len(), 6);
    // The environment value the live session set is a secret candidate, so it is
    // never written to disk.
    // Every state file, the base and each log, must be free of it.
    let mut scanned = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with("state.") {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            assert!(
                !text.contains("hello"),
                "an environment value reached {:?}",
                entry.file_name()
            );
            scanned += 1;
        }
    }
    assert!(scanned > 0, "there are state files to check");

    // The sleep snapshot forks into a running session with the same contents.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/snapshots/{sleep_snapshot}/fork"),
        &key,
        Some(json!({"count": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // Waking the sleeping session restores it under the same ID, with its file.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{sleeper}/wake"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json_of(&body)["status"], "running");
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{sleeper}/fs/keep.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"kept across the restart");

    let _ = std::fs::remove_dir_all(&dir);
}
