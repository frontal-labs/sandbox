//! Parked sessions still count against the key's session limit, although they hold no table
//! entry. This runs in its own binary, because the blob store is process-wide.

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
async fn parked_sessions_still_count_against_the_key_limit() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-parked-limit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");

    // Each sleep takes a snapshot, and a key may hold 32 snapshots, so half the allowance
    // sleeps. The other half stays live. The parked half must still count.
    let parked = sandbox_server::api::MAX_SNAPSHOTS_PER_KEY;
    for _ in 0..parked {
        let id = create(&app, &key).await;
        let (status, body) = call(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/sleep"),
            &key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    }
    for _ in parked..sandbox_server::api::MAX_SESSIONS_PER_KEY {
        create(&app, &key).await;
    }
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, Some(config())).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        String::from_utf8_lossy(&body).contains("holds 64 sessions"),
        "{}",
        String::from_utf8_lossy(&body)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
