//! `GET /metrics`: Prometheus totals for the server. It needs an API key, and it sums every
//! key's counters, so the output carries no tenant.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn send(app: &axum::Router, key: Option<&str>, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn metrics_need_a_key_and_count_the_server_activity() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);

    let (status, _) = send(&app, None, "GET", "/metrics", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = send(&app, Some(&key), "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let session: Value = serde_json::from_str(&created).unwrap();
    let id = session["session_id"].as_str().unwrap();
    let (status, _) = send(
        &app,
        Some(&key),
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["echo", "hi"], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, text) = send(&app, Some(&key), "GET", "/metrics", None).await;
    assert_eq!(status, StatusCode::OK);
    for line in [
        "sandbox_sessions_created_total 1",
        "sandbox_execs_total 1",
        "sandbox_sessions_active 1",
        "sandbox_snapshots_active 0",
    ] {
        assert!(text.lines().any(|l| l == line), "missing {line:?} in:\n{text}");
    }
    assert!(text.contains("# TYPE sandbox_execs_total counter"), "{text}");
}
