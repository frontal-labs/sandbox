//! Contract checks for the REST API: the committed spec matches the code, its
//! references resolve, and the router and the spec describe the same routes.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router, openapi};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

fn committed(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../openapi/v1")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Every `$ref` target in the document, by component name.
fn refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "$ref" {
                    if let Some(target) = child.as_str() {
                        out.push(target.rsplit('/').next().unwrap_or(target).to_string());
                    }
                } else {
                    refs(child, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| refs(item, out)),
        _ => {}
    }
}

#[test]
fn committed_json_matches_the_code() {
    assert_eq!(
        committed("openapi.json"),
        openapi::json().unwrap(),
        "openapi/v1/openapi.json is stale; run `cargo run -p sandbox-server --bin gen_openapi`"
    );
}

#[test]
fn committed_yaml_matches_the_code() {
    assert_eq!(
        committed("openapi.yaml"),
        openapi::yaml().unwrap(),
        "openapi/v1/openapi.yaml is stale; run `cargo run -p sandbox-server --bin gen_openapi`"
    );
}

#[test]
fn every_reference_resolves_to_a_component() {
    let doc: Value = serde_json::from_str(&openapi::json().unwrap()).unwrap();
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    let mut found = Vec::new();
    refs(&doc, &mut found);
    assert!(!found.is_empty());
    for target in found {
        assert!(schemas.contains_key(&target), "dangling $ref to {target}");
    }
}

#[test]
fn every_documented_operation_requires_auth_and_documents_401() {
    let doc: Value = serde_json::from_str(&openapi::json().unwrap()).unwrap();
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, op) in methods.as_object().unwrap() {
            let responses = op["responses"].as_object().unwrap();
            assert!(responses.contains_key("401"), "{method} {path} lacks 401");
            assert!(
                op["security"].is_array(),
                "{method} {path} lacks the bearer requirement"
            );
        }
    }
}

fn concrete(path: &str) -> String {
    path.replace("{id}", "01ARZ3NDEKTSV4RRFFQ69G5FAV")
        .replace("{path}", "dir/file.txt")
}

async fn send(app: axum::Router, method: &str, uri: &str, auth: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = auth {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    app.oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

async fn send_full(
    app: axum::Router,
    method: &str,
    uri: &str,
    auth: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = auth {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, body)
}

fn app(auth: Arc<AuthManager>) -> axum::Router {
    create_router(Arc::new(Scheduler::new()), auth)
}

#[tokio::test]
async fn router_and_spec_describe_the_same_routes() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let doc: Value = serde_json::from_str(&openapi::json().unwrap()).unwrap();

    for (path, methods) in doc["paths"].as_object().unwrap() {
        for method in methods.as_object().unwrap().keys() {
            let uri = concrete(path);
            let upper = method.to_uppercase();

            // Routed and authenticated: a real handler answers. Axum's route miss is
            // an empty 404, while handler errors carry a JSON body.
            let (status, body) = send_full(app(auth.clone()), &upper, &uri, Some(&key)).await;
            assert!(
                status != StatusCode::NOT_FOUND || !body.is_empty(),
                "{upper} {path} is not routed as documented"
            );
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{upper} {path} rejected the key"
            );

            // Routed but unauthenticated: refused by the auth layer.
            let status = send(app(auth.clone()), &upper, &uri, None).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{upper} {path} is not behind auth"
            );
        }
    }
}

#[tokio::test]
async fn the_spec_itself_is_public_and_matches_the_document() {
    let auth = Arc::new(AuthManager::new());
    let response = app(auth)
        .oneshot(
            Request::builder()
                .uri("/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let served: Value = serde_json::from_slice(&body).unwrap();
    let expected: Value = serde_json::from_str(&openapi::json().unwrap()).unwrap();
    assert_eq!(served, expected);
}

#[tokio::test]
async fn malformed_and_revoked_keys_are_refused() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let uri = "/v1/sessions";
    assert_eq!(
        send(app(auth.clone()), "POST", uri, Some("sbx_bad_key")).await,
        StatusCode::UNAUTHORIZED
    );

    let id = auth.validate_api_key(&key).unwrap();
    assert!(auth.revoke(&id));
    assert_eq!(
        send(app(auth.clone()), "POST", uri, Some(&key)).await,
        StatusCode::UNAUTHORIZED
    );
}
