//! The billing event stream across a restart (PLAN §4.14, M5). Needs a Postgres database
//! (SANDBOX_TEST_DATABASE_URL, used only for tests) and an S3-compatible service, as
//! `clustered_blobs.rs` does. `docker/clustered-test.sh` starts both.
#![cfg(feature = "clustered")]

use axum::http::StatusCode;
use clustered_support::{call, env, new_session, remote};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_clustered_with_remote;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

mod clustered_support;

/// Every event of the key after `after`, as JSON, in stream order.
async fn events_after(app: &axum::Router, key: &str, after: u64) -> Vec<Value> {
    let (status, body) = call(
        app,
        key,
        "GET",
        &format!("/v1/billing/events?after={after}&limit=1000"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    String::from_utf8(body)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn run(app: &axum::Router, key: &str, session: &str, command: &str) {
    let (status, body) = call(
        app,
        key,
        "POST",
        &format!("/v1/sessions/{session}/exec"),
        Some(json!({"argv": ["sh", "-c", command], "env": {}, "stream": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

async fn open(url: &str, dir: &std::path::Path) -> (axum::Router, String) {
    let (app, _state, new_key) =
        open_clustered_with_remote(Arc::new(Scheduler::new()), url, dir, None, Some(remote()))
            .await
            .unwrap();
    // The first open creates the key. A later open reads the key the first one made.
    (app, new_key.unwrap_or_default())
}

#[tokio::test]
async fn no_billing_event_is_lost_across_a_restart_and_a_consumer_resumes() {
    let url = env("SANDBOX_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS sandbox_state, billing_events")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let dir: PathBuf =
        std::env::temp_dir().join(format!("sandbox-clustered-billing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, key) = open(&url, &dir).await;
    let session = new_session(&app, &key).await;
    run(&app, &key, &session, "echo one").await;
    run(&app, &key, &session, "echo two").await;

    // The consumer reads what it has, and saves the position of the last event it handled.
    let before = events_after(&app, &key, 0).await;
    let kinds: Vec<&str> = before.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["session.started", "usage.reading", "usage.reading"],
        "{before:?}"
    );
    let saved = before.last().unwrap()["seq"].as_u64().unwrap();

    // Restart the server in the middle of the session, on the same database.
    drop(app);
    // The key made before the restart is still valid after it.
    let (app, _) = open(&url, &dir).await;

    // The session still runs, and its meter came back from the stream: two commands before the
    // restart, so the export shows three after one more.
    run(&app, &key, &session, "echo three").await;
    let (status, body) = call(&app, &key, "GET", "/v1/usage/export", None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let line: Value = serde_json::from_str(
        String::from_utf8(body)
            .unwrap()
            .lines()
            .last()
            .expect("a snapshot"),
    )
    .unwrap();
    let meter = line["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_id"] == session.as_str())
        .expect("the session is reported")["meter"]
        .clone();
    assert_eq!(
        meter["execs"], 3,
        "the restored meter kept its two commands: {meter}"
    );

    // Stopping the session writes its last event.
    let (status, _) = call(
        &app,
        &key,
        "DELETE",
        &format!("/v1/sessions/{session}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Nothing was lost: every event is there, in order, with increasing positions.
    let all = events_after(&app, &key, 0).await;
    let kinds: Vec<&str> = all.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "session.started",
            "usage.reading",
            "usage.reading",
            "usage.reading",
            "session.stopped"
        ],
        "{all:?}"
    );
    let seqs: Vec<u64> = all.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "positions increase: {seqs:?}"
    );

    // A consumer that saved a position resumes after it: exactly the events it had not seen.
    let resumed = events_after(&app, &key, saved).await;
    let kinds: Vec<&str> = resumed
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["usage.reading", "session.stopped"], "{resumed:?}");
    assert!(resumed[0]["seq"].as_u64().unwrap() > saved);
}
