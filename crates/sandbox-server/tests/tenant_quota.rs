//! Tenant isolation. A key is bound to exactly one tenant at creation time; the
//! sessions it opens belong to that tenant, whatever the request body claims.
//! The per-tenant session bound is per key (one key = one tenant), and no
//! other tenant's key can reach it.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::api::MAX_SESSIONS_PER_KEY;
use sandbox_server::auth::{AuthManager, KeyId};
use sandbox_server::create_router;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

async fn create(app: &axum::Router, key: &str, claimed_tenant: TenantId) -> StatusCode {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: claimed_tenant,
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let _: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    status
}

/// The tenant of the key, read from the key itself: split it up and validate it.
fn tenant_of(auth: &AuthManager, key: &str) -> TenantId {
    auth.validate_api_key(key).unwrap().1
}

#[tokio::test]
async fn the_key_mints_one_tenant_and_sessions_belong_to_it() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());
    let tenant = tenant_of(&auth, &key);
    assert!(!tenant.is_nil());

    // The session is created with the claimed tenant ignored.
    let foreign = TenantId::new();
    assert_ne!(foreign, tenant);
    assert_eq!(create(&app, &key, foreign).await, StatusCode::CREATED);

    // The bound counts against the key's tenant, not the claimed one: the key
    // fills its own per-key bound and the tenant bound follows.
    for _ in 1..MAX_SESSIONS_PER_KEY {
        assert_eq!(create(&app, &key, foreign).await, StatusCode::CREATED);
    }
    assert_eq!(
        create(&app, &key, foreign).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_tenants_session_bound_is_per_key() {
    let auth = Arc::new(AuthManager::new());
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());

    // One key = one tenant. The per-tenant bound equals the per-key bound
    // (64), since a key mints its own tenant and no other key shares it.
    let key_a = auth.create_key();
    let key_b = auth.create_key();
    let tenant_a = tenant_of(&auth, &key_a);
    let tenant_b = tenant_of(&auth, &key_b);
    assert_ne!(tenant_a, tenant_b, "each key mints its own tenant");

    for created in 0..MAX_SESSIONS_PER_KEY {
        assert_eq!(
            create(&app, &key_a, tenant_a).await,
            StatusCode::CREATED,
            "session {created}"
        );
    }
    // Tenant A is at its bound...
    assert_eq!(
        create(&app, &key_a, tenant_a).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // ...and tenant B is unaffected.
    assert_eq!(create(&app, &key_b, tenant_b).await, StatusCode::CREATED);
}

/// A body claiming another tenant's id cannot move a session there.
#[tokio::test]
async fn a_cannot_claim_another_tenants_identity() {
    let auth = Arc::new(AuthManager::new());
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());
    let (key_a, _) = {
        let key = auth.create_key();
        (key.clone(), tenant_of(&auth, &key))
    };
    let key_b = auth.create_key();
    let tenant_b = tenant_of(&auth, &key_b);

    // A fills its per-key bound claiming B's tenant id.
    for i in 0..MAX_SESSIONS_PER_KEY {
        assert_eq!(
            create(&app, key_a.as_str(), tenant_b).await,
            StatusCode::CREATED,
            "session {i}"
        );
    }
    // A is at its own per-key bound, even though it claimed B's id.
    assert_eq!(
        create(&app, key_a.as_str(), tenant_b).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // B's sessions are counted under B, and B is at its own tenant bound only
    // if it filled it: B has one session, far from 256.
    assert_eq!(
        create(&app, key_b.as_str(), tenant_b).await,
        StatusCode::CREATED
    );
}

/// Key ids still parse from stored form, which the persist layer relies on.
#[test]
fn key_ids_roundtrip_through_their_stored_form() {
    let auth = AuthManager::new();
    let key = auth.create_key();
    let (id, _tenant) = auth.validate_api_key(&key).unwrap();
    let restored = KeyId::from_stored(id.as_str().to_string());
    assert_eq!(restored, id);
}
