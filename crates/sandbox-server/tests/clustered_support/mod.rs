//! Helpers for the clustered tests that drive a server over HTTP. Each test binary
//! includes this module, so the helpers live once.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use object_store::aws::AmazonS3Builder;
use object_store::ObjectStore;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

pub fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}; see docker/clustered-test.sh"))
}

/// The S3-compatible service the clustered tests use (SANDBOX_S3_ENDPOINT, SANDBOX_S3_BUCKET).
pub fn remote() -> Arc<dyn ObjectStore> {
    let endpoint = env("SANDBOX_S3_ENDPOINT");
    Arc::new(
        AmazonS3Builder::from_env()
            .with_endpoint(endpoint.clone())
            .with_bucket_name(env("SANDBOX_S3_BUCKET"))
            .with_allow_http(endpoint.starts_with("http://"))
            .with_virtual_hosted_style_request(false)
            .build()
            .expect("the object store opens"),
    )
}

/// One request to the router, as `key`. Returns the status and the body bytes.
pub async fn call(
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

/// Creates an isolate session for `key` and returns its ID.
pub async fn new_session(app: &axum::Router, key: &str) -> String {
    let config = json!({
        "tenant_id": "00000000000000000000000000",
        "tier_preference": "isolate",
        "limits": {},
        "network": {"default_deny": true, "rules": []},
        "env": {},
        "cwd": "/work",
        "labels": {},
    });
    let (status, body) = call(app, key, "POST", "/v1/sessions", Some(config)).await;
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
