//! A crash during a log append leaves an incomplete last line. The store drops that line, and
//! keeps every complete record, so nothing the server acknowledged is lost.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sandbox-chaos-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn call(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Body,
) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn an_incomplete_last_record_is_dropped_and_acknowledged_state_is_kept() {
    let dir = fresh_dir("torn");
    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.unwrap();
    let config = serde_json::to_string(&SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(&app, &key, "POST", "/v1/sessions", Body::from(config)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        &key,
        "PUT",
        &format!("/v1/sessions/{id}/fs/kept.txt"),
        Body::from("acknowledged"),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    drop(app);

    // The crash: a record was being written when the process died, so its line has no newline.
    let log = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().ends_with(".log"))
        .expect("the store writes a log");
    let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    file.write_all(br#"{"op":"put_session","value":{"id":"torn"#)
        .unwrap();
    drop(file);

    let (restarted, _) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .expect("a torn tail is not damage");
    let (status, body) = call(
        &restarted,
        &key,
        "GET",
        &format!("/v1/sessions/{id}/fs/kept.txt"),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"acknowledged");
    let _ = std::fs::remove_dir_all(&dir);
}
