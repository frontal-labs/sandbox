//! Usage export: a snapshot line carries the caller's counters and the server's start time.

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
    key: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
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
        .unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn the_export_carries_the_callers_counters_and_the_server_epoch() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&created).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["echo", "billed"], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(&app, &key, "GET", "/v1/usage/export", None).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    let line: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(line["counters"]["execs"], 1, "{line}");
    assert_eq!(line["counters"]["sessions_created"], 1, "{line}");
    assert!(line["server_epoch_ms"].as_u64().unwrap() > 0, "{line}");
    assert!(line["at_ms"].as_u64().unwrap() >= line["server_epoch_ms"].as_u64().unwrap());
}
