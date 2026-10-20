//! Idle sleep (PLAN §4.14, ADR 0015), driven through the real router. One test
//! per binary: the scheduler sweeps every session in the process, so a sweep in
//! one test could put another test's sessions to sleep.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router_with_state, idle};
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    key: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
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

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

fn exec(argv: &[&str], timeout_ms: Option<u64>) -> Value {
    json!({"argv": argv, "env": {}, "cwd": null, "stdin": null, "timeout_ms": timeout_ms, "stream": false})
}

#[tokio::test]
async fn an_idle_session_sleeps_and_the_next_request_wakes_it() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, state) = create_router_with_state(Arc::new(Scheduler::new()), auth);

    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(&app, "POST", "/v1/sessions", &key, Some(config)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = json_of(&body)["session_id"].as_str().unwrap().to_string();
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        Some(exec(&["touch", "/keep.txt"], None)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // A command holds the session's gate for its whole run. A sweep must skip
    // the session while the command runs, not wait for it.
    let busy = {
        let app = app.clone();
        let key = key.clone();
        let id = id.clone();
        tokio::spawn(async move {
            call(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/exec"),
                &key,
                // An unbounded loop outlasts the 1500 ms timeout on any machine.
                // A bounded one can finish first on a fast one. The runtime's
                // own 5 s limit stops the loop.
                Some(exec(&["node", "-e", "while (true) {}"], Some(1500))),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        idle::sweep(&state, Duration::ZERO).await,
        0,
        "a busy session must not be put to sleep"
    );
    let (status, _) = busy.await.unwrap();
    assert_eq!(
        status,
        StatusCode::GATEWAY_TIMEOUT,
        "the command timed out as set"
    );

    // Now the session is idle. Its last request was a moment ago, so the sweep
    // needs a threshold that the session has passed.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(idle::sweep(&state, Duration::from_millis(20)).await, 1);
    let (status, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body)["status"], "sleeping");

    // The next request wakes the session.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        Some(exec(&["echo", "awake"], None)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json_of(&body)["stdout"], json!([97, 119, 97, 107, 101, 10]));
    // The file made before the sleep is still there after the wake.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        &key,
        Some(exec(&["cat", "/keep.txt"], None)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_of(&body)["exit_code"],
        0,
        "the file must survive the sleep"
    );
    let (_, body) = call(&app, "GET", &format!("/v1/sessions/{id}"), &key, None).await;
    assert_eq!(json_of(&body)["status"], "running");

    // The usage counters record the sleep and the wake.
    let (status, body) = call(&app, "GET", "/v1/usage", &key, None).await;
    assert_eq!(status, StatusCode::OK);
    let usage = json_of(&body);
    assert_eq!(usage["sessions_slept"], 1);
    assert_eq!(usage["sessions_woken"], 1);
    assert_eq!(usage["sessions_created"], 1);
    assert!(usage["execs"].as_u64().unwrap() >= 2);
}
