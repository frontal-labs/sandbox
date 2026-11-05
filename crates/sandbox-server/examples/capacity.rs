//! Measures the M3 acceptance numbers against the real router, in process: snapshot and
//! fork latency, and the memory of many idle sessions. It does not use the load driver in
//! `benchmarks/`, which the benchmark README marks as a scaffold.
//!
//!   cargo run --release -p sandbox-server --example capacity -- [forks] [sessions] [data-dir]
//!
//! Without a data directory the store is in memory. With one, the server stores its state on
//! disk, which is the mode where sleeping sessions can leave memory behind.
//!
//! The output is the measurement. Nothing is saved as a baseline.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

type Router = axum::Router;

async fn send(
    app: &Router,
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

fn session_body(tenant: TenantId) -> Value {
    serde_json::to_value(SessionConfig {
        tenant_id: tenant,
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap()
}

/// Resident set size of this process, in KiB, from `ps`.
fn rss_kib() -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index]
}

fn report(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    println!(
        "{name}: n={} p50={:.3} ms p95={:.3} ms max={:.3} ms",
        samples.len(),
        percentile(&samples, 0.50).as_secs_f64() * 1e3,
        percentile(&samples, 0.95).as_secs_f64() * 1e3,
        samples.last().unwrap().as_secs_f64() * 1e3,
    );
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let forks: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(200);
    let sessions: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(1000);

    // Snapshot and fork. A key may hold 32 snapshots, so each iteration uses its own key
    // and source session, which holds a file so that the snapshot has content.
    // With a directory as the third argument, the server is durable: sleeping sessions are
    // stored on disk, and that changes what sleep leaves in memory.
    let data_dir = args.next().map(std::path::PathBuf::from);
    let (app, auth) = match &data_dir {
        Some(dir) => {
            let (app, state, _) =
                sandbox_server::open_durable_with_state(Arc::new(Scheduler::new()), dir)
                    .await
                    .expect("the durable store opens");
            (app, state.auth.clone())
        }
        None => {
            let auth = Arc::new(AuthManager::new());
            let (app, _) =
                sandbox_server::create_router_with_state(Arc::new(Scheduler::new()), auth.clone());
            (app, auth)
        }
    };
    println!(
        "mode: {}",
        data_dir
            .as_deref()
            .map_or("in memory".to_string(), |d| format!(
                "durable at {}",
                d.display()
            ))
    );
    let mut snapshot_times = Vec::new();
    let mut fork_times = Vec::new();
    for _ in 0..forks {
        let key = auth.create_key();
        let (status, created) = send(
            &app,
            &key,
            "POST",
            "/v1/sessions",
            Some(session_body(TenantId::new())),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let source = created["session_id"].as_str().unwrap().to_string();
        let (status, _) = send(
            &app,
            &key,
            "POST",
            &format!("/v1/sessions/{source}/exec"),
            Some(
                json!({"argv": ["sh", "-c", "echo state > file.txt"], "env": {}, "stream": false}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let started = Instant::now();
        let (status, taken) = send(
            &app,
            &key,
            "POST",
            &format!("/v1/sessions/{source}/snapshot"),
            None,
        )
        .await;
        snapshot_times.push(started.elapsed());
        assert_eq!(status, StatusCode::CREATED, "{taken}");
        let snapshot = taken["snapshot_id"].as_str().unwrap().to_string();

        let started = Instant::now();
        let (status, forked) = send(
            &app,
            &key,
            "POST",
            &format!("/v1/snapshots/{snapshot}/fork"),
            Some(json!({"count": 1})),
        )
        .await;
        fork_times.push(started.elapsed());
        assert!(status.is_success(), "{forked}");
    }
    report("snapshot", snapshot_times);
    report("fork (one session)", fork_times);

    // Create and run a command. Each session gets its own key, so the quotas do not interfere.
    let mut create_times = Vec::new();
    let mut exec_times = Vec::new();
    for _ in 0..50 {
        let key = auth.create_key();
        let started = Instant::now();
        let (status, created) = send(
            &app,
            &key,
            "POST",
            "/v1/sessions",
            Some(session_body(TenantId::new())),
        )
        .await;
        create_times.push(started.elapsed());
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["session_id"].as_str().unwrap().to_string();
        let started = Instant::now();
        let (status, ran) = send(
            &app,
            &key,
            "POST",
            &format!("/v1/sessions/{id}/exec"),
            Some(json!({"argv": ["echo", "hi"], "env": {}, "stream": false})),
        )
        .await;
        exec_times.push(started.elapsed());
        assert_eq!(status, StatusCode::OK, "{ran}");
    }
    report("create session", create_times);
    report("exec echo", exec_times);

    // Idle sessions: memory per session, measured as the growth in resident memory.
    // A key holds 64 sessions and 32 snapshots, so keys take 30 sessions each.
    let before = rss_kib();
    let started = Instant::now();
    let mut key = auth.create_key();
    let mut tenant = TenantId::new();
    let mut ids = Vec::with_capacity(sessions);
    let mut keys = Vec::new();
    for index in 0..sessions {
        if index > 0 && index % 30 == 0 {
            key = auth.create_key();
            tenant = TenantId::new();
        }
        keys.push(key.clone());
        let (status, created) = send(
            &app,
            &key,
            "POST",
            "/v1/sessions",
            Some(session_body(tenant)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "session {index}: {created}");
        ids.push(created["session_id"].as_str().unwrap().to_string());
    }
    let elapsed = started.elapsed();
    let after = rss_kib();
    println!(
        "idle sessions: {sessions} created in {:.2} s; resident memory {} KiB -> {} KiB, {:.1} KiB per session",
        elapsed.as_secs_f64(),
        before,
        after,
        (after.saturating_sub(before)) as f64 / sessions as f64,
    );

    // Sleeping sessions: put every idle session to sleep, and measure the memory they hold.
    let started = Instant::now();
    for (id, key) in ids.iter().zip(&keys) {
        let (status, slept) =
            send(&app, key, "POST", &format!("/v1/sessions/{id}/sleep"), None).await;
        assert!(status.is_success(), "{slept}");
    }
    let slept_after = rss_kib();
    println!(
        "sleeping sessions: {sessions} slept in {:.2} s; resident memory {} KiB, {:.1} KiB per session above the idle baseline",
        started.elapsed().as_secs_f64(),
        slept_after,
        (slept_after as f64 - before as f64) / sessions as f64,
    );
}
