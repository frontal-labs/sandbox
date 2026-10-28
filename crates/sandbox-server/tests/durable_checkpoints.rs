//! Checkpoint roots survive a restart of a durable server, so undo still works after
//! one. One test per binary: the blob store is process-wide.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    body: Option<(&str, Vec<u8>)>,
) -> (StatusCode, Vec<u8>) {
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

fn json_call(value: &Value) -> Option<(&'static str, Vec<u8>)> {
    Some(("application/json", serde_json::to_vec(value).unwrap()))
}

fn config() -> Value {
    serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap()
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

#[tokio::test]
async fn checkpoint_roots_survive_a_restart() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-checkpoints-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = key.expect("the first run creates a key");
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, json_call(&config())).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let id = json_of(&body)["session_id"].as_str().unwrap().to_string();

    for text in ["one", "two"] {
        let (status, _) = call(
            &app,
            "PUT",
            &format!("/v1/sessions/{id}/fs/a.txt"),
            &key,
            Some(("application/octet-stream", text.as_bytes().to_vec())),
        )
        .await;
        assert!(status.is_success());
        let (status, _) = call(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/exec"),
            &key,
            json_call(&json!({"argv": ["true"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    // Restart: a new server reads the same directory.
    drop(app);
    let (app, none) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(none.is_none());

    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        json_of(&body)["checkpoints"]["count"],
        3,
        "all three roots were saved"
    );

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/undo"),
        &key,
        json_call(&json!({"steps": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/a.txt"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"one");
}
