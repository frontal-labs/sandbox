//! The usage export on a durable server keeps its history in the data directory: each export adds a
//! snapshot line, and the reply holds the caller's lines, oldest first.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn each_export_is_kept_in_the_data_directory() {
    let dir: PathBuf = std::env::temp_dir().join(format!("sandbox-billing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (app, key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = key.unwrap();
    let mut last = String::new();
    for _ in 0..2 {
        let request = Request::builder()
            .uri("/v1/usage/export")
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        last = String::from_utf8(bytes.to_vec()).unwrap();
    }
    assert_eq!(last.lines().count(), 2, "{last}");
    let kept = std::fs::read_to_string(dir.join("usage.jsonl")).unwrap();
    assert_eq!(kept.lines().count(), 2, "{kept}");
    let _ = std::fs::remove_dir_all(&dir);
}
