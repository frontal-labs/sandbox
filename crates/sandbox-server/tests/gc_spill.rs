//! Output kept as a blob, because the trace was full, survives a collection and
//! stays readable. This is its own binary: the blob store is process-wide.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_session::blob_store;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
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

#[tokio::test]
async fn a_collection_keeps_spilled_output_readable() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, json_call(&config)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let big: Vec<u8> = (0..200_000)
        .flat_map(|i| format!("{i:08}\n").into_bytes())
        .collect();
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/big.txt"),
        &key,
        Some(("application/octet-stream", big.clone())),
    )
    .await;
    assert!(status.is_success());
    let run = json!({"argv": ["sh", "-c", "cat /big.txt; echo end"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false});
    let mut last = Value::Null;
    for _ in 0..5 {
        let (status, body) = call(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/exec"),
            &key,
            json_call(&run),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        last = serde_json::from_slice(&body).unwrap();
    }
    let stdout = String::from_utf8(
        last["stdout"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap() as u8)
            .collect(),
    )
    .unwrap();
    let path = stdout
        .lines()
        .find(|line| line.starts_with("…["))
        .and_then(|line| line.split("full: ").nth(1))
        .map(|rest| rest.trim_end_matches(']').to_string())
        .expect("the fifth run is capped with a path");

    let orphan = blob_store()
        .put(b"no root refers to this", Default::default())
        .await
        .unwrap();
    let (status, _) = call(&app, "POST", "/v1/gc", &key, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while blob_store().has(&orphan).await.unwrap() {
        assert!(
            std::time::Instant::now() < deadline,
            "the orphan was not swept"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let (status, full) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/fs/{}", path.trim_start_matches('/')),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&full));
    // The output differs from the file, so its blob is not already kept by the file's tree.
    let mut expected = big.clone();
    expected.extend_from_slice(b"end\n");
    assert!(
        full == expected,
        "the spilled stream survives the collection"
    );
}
