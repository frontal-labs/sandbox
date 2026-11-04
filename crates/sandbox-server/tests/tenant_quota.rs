//! The per-tenant session bound: keys of one tenant share it, and another tenant is not
//! affected by it.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::api::MAX_SESSIONS_PER_TENANT;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

async fn create(app: &axum::Router, key: &str, tenant: TenantId) -> StatusCode {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: tenant,
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(config.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let _: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    status
}

#[tokio::test]
async fn a_tenants_keys_share_one_session_bound() {
    let auth = Arc::new(AuthManager::new());
    // Four keys, each at its own bound of 64, make up the tenant's 256.
    let keys: Vec<String> = (0..4).map(|_| auth.create_key()).collect();
    let other_key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let tenant = TenantId::new();

    for created in 0..MAX_SESSIONS_PER_TENANT {
        let key = &keys[created / 64];
        assert_eq!(create(&app, key, tenant).await, StatusCode::CREATED, "session {created}");
    }
    assert_eq!(create(&app, &keys[3], tenant).await, StatusCode::TOO_MANY_REQUESTS);

    // Another tenant is unaffected.
    assert_eq!(create(&app, other_key.as_str(), TenantId::new()).await, StatusCode::CREATED);
}
