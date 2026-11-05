//! An unpinned checkpoint's blobs are reclaimed by the next collection. This is its
//! own binary: the blob store is process-wide, and a collection in one test would
//! sweep blobs that another test's server still references.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_session::blob_store;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
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
async fn an_unpinned_checkpoint_is_reclaimed_by_the_next_collection() {
    use sandbox_types::hash::ContentHash;

    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        checkpoint_retain: 1,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, json_call(&config)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Pin a checkpoint that holds "gone", then write "kept" and pin again. Retention of
    // one unpins the first checkpoint.
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/a.txt"),
        &key,
        Some(("application/octet-stream", b"gone".to_vec())),
    )
    .await;
    assert!(status.is_success());
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/checkpoint"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let unpinned_root = ContentHash::from_hex(
        serde_json::from_slice::<Value>(&body).unwrap()["root"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/v1/sessions/{id}/fs/a.txt"),
        &key,
        Some(("application/octet-stream", b"kept".to_vec())),
    )
    .await;
    assert!(status.is_success());
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/checkpoint"),
        &key,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        blob_store().has(&unpinned_root).await.unwrap(),
        "present before the collection"
    );

    let orphan = blob_store()
        .put(b"no root refers to this either", Default::default())
        .await
        .unwrap();
    let (status, _) = call(&app, "POST", "/v1/gc", &key, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while blob_store().has(&orphan).await.unwrap() {
        assert!(
            std::time::Instant::now() < deadline,
            "the orphan was not swept"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(
        !blob_store().has(&unpinned_root).await.unwrap(),
        "the unpinned checkpoint's root was reclaimed"
    );
}
