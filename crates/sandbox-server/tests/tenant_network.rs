//! The tenant network policy is checked at session creation (PLAN §multi-tenancy,
//! item 4). The server's default tenant is `allow_all`, which admits every
//! (host, port) a session could ask for, so every session is admitted by
//! default. The reject direction — a session that exceeds a restrictive tenant
//! policy is refused — is covered by the `is_superset_of` unit tests in
//! `sandbox-types/src/policy.rs`, which exercise the same predicate the server
//! calls.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::policy::{NetworkPolicy, NetworkPolicyRule};
use sandbox_types::{SessionConfig, TierPreference};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn create_request(key: &str, network: &NetworkPolicy) -> Request<Body> {
    let config = SessionConfig {
        network: network.clone(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    };
    let body = serde_json::to_value(&config).unwrap();
    Request::builder()
        .method("POST")
        .uri("/v1/sessions")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn post(app: &axum::Router, key: &str, network: &NetworkPolicy) -> StatusCode {
    let response = app
        .clone()
        .oneshot(create_request(key, network))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    if !status.is_success() {
        let body_str = String::from_utf8_lossy(&bytes);
        eprintln!("  response {status}: {body_str}");
    }
    let _: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    status
}

/// The default (empty, default-deny) session policy asks for nothing, so the
/// default `allow_all` tenant covers it.
#[tokio::test]
async fn a_default_deny_session_is_admitted_by_the_default_tenant() {
    let auth = Arc::new(AuthManager::new());
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());
    let key = auth.create_key();
    assert_eq!(
        post(&app, &key, &NetworkPolicy::default()).await,
        StatusCode::CREATED
    );
}

/// A session that names a specific host is admitted when the default
/// `allow_all` tenant covers it.
#[tokio::test]
async fn a_named_host_session_is_admitted_by_the_default_tenant() {
    let auth = Arc::new(AuthManager::new());
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());
    let key = auth.create_key();

    let mut session = NetworkPolicy::default();
    session.add_rule(NetworkPolicyRule::allow_http("api.example.com"));
    assert_eq!(post(&app, &key, &session).await, StatusCode::CREATED);
}

/// A default-open session admits every host and port; the default `allow_all`
/// tenant covers it.
#[tokio::test]
async fn a_default_open_session_is_admitted_by_the_default_tenant() {
    let auth = Arc::new(AuthManager::new());
    let app = create_router(Arc::new(Scheduler::new()), auth.clone());
    let key = auth.create_key();
    assert_eq!(
        post(&app, &key, &NetworkPolicy::allow_all()).await,
        StatusCode::CREATED
    );
}
