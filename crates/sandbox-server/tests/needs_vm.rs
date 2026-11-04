//! A command the isolate tier cannot run. Without a guest image the server refuses it with
//! `NEEDS_VM` (503). A session created with `isolate` never moves to a VM, even with an
//! image configured. The promotion itself needs QEMU and a guest image, and is covered by
//! `vm_fallback.rs`, which is ignored unless the image is given.

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
) -> (StatusCode, Value) {
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn session(app: &axum::Router, key: &str, preference: TierPreference) -> String {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: preference,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = call(app, key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["session_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_command_the_isolate_tier_lacks_is_refused_as_needs_vm_without_an_image() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let id = session(&app, &key, TierPreference::Auto).await;

    let (status, body) = call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["nonexistent-tool"], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "NEEDS_VM", "{body}");
}
