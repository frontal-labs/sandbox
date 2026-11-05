//! A new session runs CPython, installed by the server from the build artifacts. Ignored unless
//! SANDBOX_PYTHON_DIR names the directory tools/wasm/python/build.sh writes to.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router_with_python;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use sandbox_wasm::PythonArtifacts;
use serde_json::{json, Value};
use std::path::PathBuf;
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

#[tokio::test]
#[ignore = "needs SANDBOX_PYTHON_DIR; see tools/wasm/python/build.sh"]
async fn a_new_session_runs_cpython_installed_by_the_server() {
    let dir = PathBuf::from(std::env::var("SANDBOX_PYTHON_DIR").expect("set SANDBOX_PYTHON_DIR"));
    let artifacts = PythonArtifacts::load(&dir).unwrap();
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, _state) = create_router_with_python(Arc::new(Scheduler::new()), auth, artifacts);

    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["session_id"].as_str().unwrap().to_string();

    let (status, out) = call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["python", "-c", "import json; print(json.dumps({'answer': 6 * 7}))"], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["exit_code"], 0, "{out}");
    let stdout: Vec<u8> = out["stdout"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(String::from_utf8(stdout).unwrap(), "{\"answer\": 42}\n");
}
