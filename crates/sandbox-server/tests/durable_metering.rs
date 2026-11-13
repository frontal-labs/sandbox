//! Billing metering on a durable single-node server (PLAN §4.14, M5). Meters are
//! in memory, so after a restart they must come back from the billing stream
//! (`billing.jsonl` in the data directory), which kept every reading. A session
//! that stopped keeps its final meter, and a session that never ran a command
//! is still reported with a zero meter.
//!
//! Both scenarios run in one test function because the blob store is a
//! process-level singleton: a second `open_durable` with a different directory
//! would fail.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

/// Sends an authenticated JSON request, returning status and body bytes.
async fn get(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    payload: Option<&Value>,
) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match payload {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let code = res.status();
    let raw = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (code, raw.to_vec())
}

/// Creates a session and returns its ID.
async fn new_session(app: &axum::Router, key: &str) -> String {
    let cfg = SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..Default::default()
    };
    let (code, raw) = get(
        app,
        key,
        "POST",
        "/v1/sessions",
        Some(&serde_json::to_value(&cfg).unwrap()),
    )
    .await;
    assert_eq!(
        code,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&raw)
    );
    let v: Value = serde_json::from_slice(&raw).unwrap();
    v["session_id"].as_str().unwrap().to_string()
}

/// Runs a command in a session and asserts it succeeds.
async fn exec(app: &axum::Router, key: &str, sid: &str, cmd: &str) {
    let body = json!({"argv": ["sh", "-c", cmd], "env": {}, "stream": false});
    let (code, raw) = get(
        app,
        key,
        "POST",
        &format!("/v1/sessions/{sid}/exec"),
        Some(&body),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{}", String::from_utf8_lossy(&raw));
}

/// Uploads a file into a session and asserts 204.
async fn put_file(app: &axum::Router, key: &str, sid: &str, rel: &str, data: &str) {
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/sessions/{sid}/fs/{rel}"))
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(data.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let code = res.status();
    let raw = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        code,
        StatusCode::NO_CONTENT,
        "upload {rel} failed: {}",
        String::from_utf8_lossy(&raw)
    );
}

/// Returns the last snapshot line from the usage export endpoint.
async fn export_snapshot(app: &axum::Router, key: &str) -> Value {
    let (code, raw) = get(app, key, "GET", "/v1/usage/export", None).await;
    assert_eq!(code, StatusCode::OK, "{}", String::from_utf8_lossy(&raw));
    let text = String::from_utf8(raw).unwrap();
    serde_json::from_str(text.lines().last().expect("export has a snapshot")).unwrap()
}

fn find_meter(line: &Value, session_id: &str) -> Value {
    let sessions = line["sessions"].as_array().expect("sessions are listed");
    sessions
        .iter()
        .find(|s| s["session_id"] == session_id)
        .unwrap_or_else(|| panic!("session {session_id} has no metering record: {line}"))["meter"]
        .clone()
}

/// Meters survive a restart: the billing stream in the data directory is replayed
/// to restore every session's meter. A busy session keeps its readings, an idle
/// session is still reported with a zero meter, and a stopped session keeps its
/// final meter and stays stopped.
#[tokio::test]
async fn meters_survive_a_restart_from_the_billing_stream() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-durable-metering-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");

    // A busy session: write some data and run two commands so the meter is non-zero.
    let busy = new_session(&app, &key).await;
    put_file(&app, &key, &busy, "data.txt", &"x".repeat(2048)).await;
    exec(&app, &key, &busy, "cat /work/data.txt | wc -c").await;
    exec(&app, &key, &busy, "echo done").await;

    // A stopped session: created, run once, then destroyed.
    let stopped = new_session(&app, &key).await;
    exec(&app, &key, &stopped, "echo stop-me").await;
    let (code, _) = get(
        &app,
        &key,
        "DELETE",
        &format!("/v1/sessions/{stopped}"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT, "session is destroyed");

    // An idle session: created, never run. It must still be reported.
    let idle = new_session(&app, &key).await;

    // Before the restart, the export shows the meters.
    let pre = export_snapshot(&app, &key).await;
    let busy_meter_pre = find_meter(&pre, &busy);
    assert_eq!(busy_meter_pre["execs"], 2, "{busy_meter_pre}");
    assert!(
        busy_meter_pre["peak_memory_bytes"].as_u64().unwrap() >= 2048,
        "memory is reported: {busy_meter_pre}"
    );
    let stopped_meter_pre = find_meter(&pre, &stopped);
    assert_eq!(stopped_meter_pre["execs"], 1, "{stopped_meter_pre}");

    // Drop the server (simulating a restart) and reopen the same data directory.
    drop(app);
    let (restarted, _) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();

    // After the restart, the meters must be restored from billing.jsonl.
    let post = export_snapshot(&restarted, &key).await;

    // The busy session's meter survives.
    let busy_meter_post = find_meter(&post, &busy);
    assert_eq!(
        busy_meter_post["execs"], 2,
        "execs survive the restart: {busy_meter_post}"
    );
    assert_eq!(
        busy_meter_post["peak_memory_bytes"], busy_meter_pre["peak_memory_bytes"],
        "peak memory survives: pre={busy_meter_pre} post={busy_meter_post}"
    );
    assert!(
        busy_meter_post["cpu_ms"].is_u64(),
        "CPU time is restored: {busy_meter_post}"
    );
    assert!(
        busy_meter_post["memory_byte_ms"].is_u64(),
        "memory-time is restored: {busy_meter_post}"
    );

    // The stopped session keeps its final meter and stays stopped.
    let stopped_meter_post = find_meter(&post, &stopped);
    assert_eq!(
        stopped_meter_post["execs"], 1,
        "the stopped session's final meter survives: {stopped_meter_post}"
    );
    assert_eq!(
        stopped_meter_post["cpu_ms"], stopped_meter_pre["cpu_ms"],
        "cpu_ms is stable: pre={stopped_meter_pre} post={stopped_meter_post}"
    );

    // The idle session is still reported, with a zero meter.
    let idle_meter = find_meter(&post, &idle);
    assert_eq!(idle_meter["execs"], 0, "{idle_meter}");
    assert_eq!(idle_meter["cpu_ms"], 0, "{idle_meter}");
    assert_eq!(idle_meter["peak_memory_bytes"], 0, "{idle_meter}");

    let _ = std::fs::remove_dir_all(&dir);
}
