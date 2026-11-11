//! Billing metering on a clustered server (PLAN §4.14, M5). Each session produces a meter
//! with CPU time and memory, and the usage export reports them. Needs a Postgres database
//! (SANDBOX_TEST_DATABASE_URL, used only for tests) and an S3-compatible service, as
//! `clustered_blobs.rs` does. `docker/clustered-test.sh` starts both.
#![cfg(feature = "clustered")]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use clustered_support::{call, env, new_session, remote};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_clustered_with_remote;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

mod clustered_support;

#[tokio::test]
async fn every_session_reports_cpu_and_memory_in_the_usage_export() {
    let url = env("SANDBOX_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS sandbox_state")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-clustered-metering-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (app, _state, new_key) =
        open_clustered_with_remote(Arc::new(Scheduler::new()), &url, &dir, None, Some(remote()))
            .await
            .unwrap();
    let key = new_key.expect("an empty database creates a key");

    // A busy session: it writes 2 KiB and runs two commands.
    let busy = new_session(&app, &key).await;
    let payload = "x".repeat(2048);
    let put = Request::builder()
        .method("PUT")
        .uri(format!("/v1/sessions/{busy}/fs/data.txt"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(payload))
        .unwrap();
    assert!(app
        .clone()
        .oneshot(put)
        .await
        .unwrap()
        .status()
        .is_success());
    for command in ["cat /work/data.txt | wc -c", "echo done"] {
        let (status, body) = call(
            &app,
            &key,
            "POST",
            &format!("/v1/sessions/{busy}/exec"),
            Some(json!({"argv": ["sh", "-c", command], "env": {}, "stream": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    }

    // An idle session: created, never run. It must still be reported.
    let idle = new_session(&app, &key).await;

    let (status, body) = call(&app, &key, "GET", "/v1/usage/export", None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let text = String::from_utf8(body).unwrap();
    let line: Value =
        serde_json::from_str(text.lines().last().expect("the export has a snapshot")).unwrap();
    let sessions = line["sessions"].as_array().expect("sessions are listed");

    let find = |id: &str| {
        sessions
            .iter()
            .find(|s| s["session_id"] == id)
            .unwrap_or_else(|| panic!("session {id} has no metering record: {line}"))["meter"]
            .clone()
    };
    let busy_meter = find(&busy);
    assert_eq!(busy_meter["execs"], 2, "{busy_meter}");
    assert!(
        busy_meter["cpu_ms"].is_u64(),
        "CPU time is reported: {busy_meter}"
    );
    assert!(
        busy_meter["peak_memory_bytes"].as_u64().unwrap() >= 2048,
        "memory is reported: {busy_meter}"
    );
    assert!(
        busy_meter["memory_byte_ms"].is_u64(),
        "memory-time is reported: {busy_meter}"
    );

    // The key's total is the sum of its sessions' meters.
    let total = &line["total"];
    assert_eq!(total["execs"], 2, "{total}");
    assert_eq!(total["cpu_ms"], busy_meter["cpu_ms"], "{total}");

    let idle_meter = find(&idle);
    assert_eq!(idle_meter["execs"], 0, "{idle_meter}");
    assert_eq!(idle_meter["cpu_ms"], 0);
    assert_eq!(idle_meter["peak_memory_bytes"], 0);
}
