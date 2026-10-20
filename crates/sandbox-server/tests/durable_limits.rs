//! Session limits across a restart (ADR 0014, ADR 0010). Its own test binary,
//! because the blob store of a durable server is installed once per process.

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
    body: &[u8],
) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(body.to_vec()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
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

#[tokio::test]
async fn a_quota_survives_a_restart() {
    let dir: PathBuf = std::env::temp_dir().join(format!("sandbox-quota-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");
    let mut quota_config = config();
    quota_config["limits"] = json!({"max_file_size": 8});
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, Some(quota_config)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let id = json_of(&body)["session_id"].as_str().unwrap().to_string();
    drop(app);

    // A restart brings the live session back with the same limit.
    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(new_key.is_none(), "the key is kept, not created again");
    let (status, _) = call_bytes(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/small.txt"),
        &key,
        b"1234",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call_bytes(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/big.txt"),
        &key,
        b"123456789",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "the restored session lost its quota"
    );
}
