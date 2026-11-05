//! Resident memory of a server that reopens a directory full of sleeping sessions.
//!
//! Run in two steps, each in its own process, so the measurement starts cold:
//!
//! ```text
//! parked_restart measure <dir>            # baseline: the directory as it is
//! parked_restart seed <dir> <sessions>    # create and sleep sessions in <dir>
//! parked_restart measure <dir>            # what a restarted server holds now
//! ```
//!
//! A sleeping session on a durable server is one file on disk, so the second measurement
//! should sit at the baseline, not above it.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable_with_state;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tower::ServiceExt;

/// Resident set size of this process, in KiB, from `ps`.
fn rss_kib() -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ps reports a number")
}

async fn send(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> StatusCode {
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
        .oneshot(builder.body(body).expect("a request builds"))
        .await
        .expect("the router answers");
    response.status()
}

async fn create_session(app: &axum::Router, key: &str, tenant: TenantId) -> String {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: tenant,
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .expect("a config encodes");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/sessions")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(config.to_string()))
        .expect("a request builds");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    let created: Value = serde_json::from_slice(&bytes).expect("the reply is JSON");
    created["session_id"].as_str().expect("an ID").to_string()
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let dir = PathBuf::from(args.next().expect("a data directory"));
    match mode.as_str() {
        "measure" => {
            let (_app, _state, _) = open_durable_with_state(Arc::new(Scheduler::new()), &dir)
                .await
                .expect("the durable store opens");
            println!("resident memory after open: {} KiB", rss_kib());
        }
        "seed" => {
            let count: usize = args
                .next()
                .and_then(|v| v.parse().ok())
                .expect("a session count");
            let (app, state, _) = open_durable_with_state(Arc::new(Scheduler::new()), &dir)
                .await
                .expect("the durable store opens");
            // A key may hold 32 snapshots, and each sleep takes one, so keys take 30 sessions.
            let mut key = state.auth.create_key();
            let mut tenant = TenantId::new();
            for index in 0..count {
                if index > 0 && index % 30 == 0 {
                    key = state.auth.create_key();
                    tenant = TenantId::new();
                }
                let id = create_session(&app, &key, tenant).await;
                let status = send(
                    &app,
                    &key,
                    "POST",
                    &format!("/v1/sessions/{id}/sleep"),
                    None,
                )
                .await;
                assert!(status.is_success(), "sleep failed: {status}");
            }
            println!("slept {count} sessions in {}", dir.display());
        }
        "snapshots" => {
            // Sessions that take a snapshot and stay awake: the same snapshot records as a
            // sleep leaves, but no parked sessions. Separates the two costs.
            let count: usize = args.next().and_then(|v| v.parse().ok()).expect("a count");
            let (app, state, _) = open_durable_with_state(Arc::new(Scheduler::new()), &dir)
                .await
                .expect("the durable store opens");
            let mut key = state.auth.create_key();
            let mut tenant = TenantId::new();
            for index in 0..count {
                if index > 0 && index % 30 == 0 {
                    key = state.auth.create_key();
                    tenant = TenantId::new();
                }
                let id = create_session(&app, &key, tenant).await;
                let status = send(
                    &app,
                    &key,
                    "POST",
                    &format!("/v1/sessions/{id}/snapshot"),
                    None,
                )
                .await;
                assert!(status.is_success(), "snapshot failed: {status}");
                // Destroy the session, so only its snapshot record remains.
                let status = send(&app, &key, "DELETE", &format!("/v1/sessions/{id}"), None).await;
                assert!(status.is_success(), "destroy failed: {status}");
            }
            println!("took {count} snapshots in {}", dir.display());
        }
        other => panic!("unknown mode {other:?}; use measure, seed, or snapshots"),
    }
}
