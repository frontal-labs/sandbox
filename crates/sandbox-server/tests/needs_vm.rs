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

#[tokio::test]
async fn a_vm_preference_is_refused_without_a_guest_image() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);

    // A session may not be created directly in the VM tier when the server has no image,
    // so the request is refused rather than silently ignored.
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Vm,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert_eq!(body["code"], "UNSUPPORTED", "{body}");

    // And an isolate preference is still fine on the same server.
    let id = session(&app, &key, TierPreference::Isolate).await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn a_streamed_command_the_isolate_tier_lacks_ends_with_needs_vm_without_an_image() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let id = session(&app, &key, TierPreference::Auto).await;

    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/sessions/{id}/exec?stream=true"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"argv": ["nonexistent-tool"], "env": {}, "stream": true}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let last: Value =
        serde_json::from_str(String::from_utf8_lossy(&bytes).lines().last().unwrap()).unwrap();
    assert_eq!(last["type"], "error", "{last}");
    assert_eq!(last["code"], "NEEDS_VM", "{last}");
}
