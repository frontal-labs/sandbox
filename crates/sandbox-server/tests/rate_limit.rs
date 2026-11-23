//! Per-API-key rate limiting, driven through the real router.
//!
//! The limiter is a fixed-window counter: a key may make at most `requests_per_second`
//! requests in any one-second window. Over-budget requests receive `429 Too Many
//! Requests` with a `Retry-After` header.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router_with_options, ServerOptions};
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    body: Option<(&str, Vec<u8>)>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some((content_type, bytes)) => {
            builder = builder.header(header::CONTENT_TYPE, content_type);
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

/// Build a router with a rate limit of `rps` requests per second.
fn router_with_limit(rps: u64) -> (axum::Router, String) {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, _state) = create_router_with_options(
        Arc::new(Scheduler::new()),
        auth,
        ServerOptions {
            rate_limit: Some(rps),
            ..Default::default()
        },
    );
    (app, key)
}

#[tokio::test]
async fn a_key_over_its_budget_receives_429_with_retry_after() {
    let (app, key) = router_with_limit(3);
    // Three requests are admitted.
    for _ in 0..3 {
        let (status, _, _) = call(&app, "POST", "/v1/sessions", &key, None).await;
        assert!(
            status != StatusCode::TOO_MANY_REQUESTS,
            "request within budget should not be 429"
        );
    }
    // The fourth is over budget.
    let (status, headers, body) = call(&app, "POST", "/v1/sessions", &key, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        headers.contains_key(header::RETRY_AFTER),
        "429 must carry a Retry-After header"
    );
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("rate limit"),
        "429 body should mention rate limit, got: {text}"
    );
}

#[tokio::test]
async fn different_keys_have_independent_budgets() {
    let auth = Arc::new(AuthManager::new());
    let key_a = auth.create_key();
    let key_b = auth.create_key();
    let (app, _state) = create_router_with_options(
        Arc::new(Scheduler::new()),
        auth,
        ServerOptions {
            rate_limit: Some(2),
            ..Default::default()
        },
    );
    // Exhaust key A's budget.
    for _ in 0..2 {
        let (status, _, _) = call(&app, "GET", "/v1/sessions", &key_a, None).await;
        assert!(status != StatusCode::TOO_MANY_REQUESTS);
    }
    let (status, _, _) = call(&app, "GET", "/v1/sessions", &key_a, None).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "key A should be throttled"
    );
    // Key B still has its full budget.
    for _ in 0..2 {
        let (status, _, _) = call(&app, "GET", "/v1/sessions", &key_b, None).await;
        assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "key B should have an independent budget"
        );
    }
}

#[tokio::test]
async fn zero_rps_rejects_every_request() {
    let (app, key) = router_with_limit(0);
    let (status, headers, _) = call(&app, "GET", "/v1/sessions", &key, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn the_default_budget_admits_many_requests() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, _state) =
        create_router_with_options(Arc::new(Scheduler::new()), auth, ServerOptions::default());
    // The default is 200 rps; 50 requests should all be admitted.
    for _ in 0..50 {
        let (status, _, _) = call(&app, "GET", "/v1/sessions", &key, None).await;
        assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "default budget should admit well under 200 requests"
        );
    }
}

#[tokio::test]
async fn an_unauthenticated_request_is_rejected_before_rate_limiting() {
    let (app, _) = router_with_limit(1);
    // No auth header: rejected by require_api_key, not by the rate limiter.
    let builder = Request::builder().method("GET").uri("/v1/sessions");
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
