//! A clustered server keeps its blobs in an object store too (ADR 0035). Needs a Postgres database
//! (SANDBOX_TEST_DATABASE_URL, used only for tests) and an S3-compatible service
//! (SANDBOX_S3_ENDPOINT, SANDBOX_S3_BUCKET). docker/clustered-test.sh starts both.
#![cfg(feature = "clustered")]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::ObjectStore;
use sandbox_scheduler::Scheduler;
use sandbox_server::open_clustered_with_remote;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}; see docker/clustered-test.sh"))
}

fn remote() -> Arc<dyn ObjectStore> {
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

async fn call(
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

#[tokio::test]
async fn a_clustered_server_reads_its_blobs_back_from_the_object_store() {
    let url = env("SANDBOX_TEST_DATABASE_URL");
    // A clean table, so the test does not see another run's state.
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS sandbox_state")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-clustered-blobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = remote();

    let (app, _state, new_key) = open_clustered_with_remote(
        Arc::new(Scheduler::new()),
        &url,
        &dir,
        None,
        Some(store.clone()),
    )
    .await
    .unwrap();
    let key = new_key.expect("an empty database creates a key");
    let config = json!({
        "tenant_id": "00000000000000000000000000",
        "tier_preference": "isolate",
        "limits": {},
        "network": {"default_deny": true, "rules": []},
        "env": {},
        "cwd": "/work",
        "labels": {},
    });
    let (status, body) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let id = serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/v1/sessions/{id}/fs/kept.txt"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from("written through the clustered object store"))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    let (status, body) = call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/snapshot"),
        None,
    )
    .await;
    assert!(status.is_success(), "{}", String::from_utf8_lossy(&body));

    // The bucket holds the blobs the server wrote.
    let mut listing = store.list(None);
    let mut objects = 0;
    while let Some(entry) = listing.next().await {
        entry.unwrap();
        objects += 1;
    }
    assert!(objects > 0, "the bucket holds the blobs");

    // A node with none of the blobs locally reads them from the bucket.
    std::fs::remove_dir_all(dir.join("blobs")).unwrap();
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let (app, _state, _) = open_clustered_with_remote(
        Arc::new(Scheduler::new()),
        &url,
        &dir,
        None,
        Some(store.clone()),
    )
    .await
    .unwrap();
    let (status, body) = call(
        &app,
        &key,
        "GET",
        &format!("/v1/sessions/{id}/fs/kept.txt"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"written through the clustered object store");
    let _ = std::fs::remove_dir_all(&dir);
}
