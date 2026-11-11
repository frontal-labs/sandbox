//! A command typed into the PTY shell is checkpointed on a durable server (ADR 0043's open
//! item): its file survives a restart, and the trace records the line that wrote it.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures::{SinkExt, StreamExt};
use sandbox_scheduler::Scheduler;
use sandbox_server::open_durable;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

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

async fn create(app: &axum::Router, key: &str) -> String {
    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, body) = call(app, key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice::<Value>(&body).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Types one line into the shell and waits for the prompt that follows its output.
async fn type_line(addr: SocketAddr, session: &str, key: &str, line: &str) {
    let url = format!("ws://{addr}/v1/sessions/{session}/pty");
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {key}").parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let read = async {
        let mut seen = String::new();
        // The prompt comes first, then the line is typed, then the prompt returns after it runs.
        socket
            .send(Message::Text(
                json!({"type": "input", "data": format!("{line}\r")})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            if event["type"] == "output" {
                seen.push_str(event["data"].as_str().unwrap());
                if seen.matches("$ ").count() >= 2 {
                    return;
                }
            }
        }
        panic!("the shell ended before the line finished: {seen:?}");
    };
    tokio::time::timeout(Duration::from_secs(20), read)
        .await
        .expect("the line runs");
    let _ = socket.close(None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_line_typed_into_the_shell_survives_a_restart() {
    let dir: std::path::PathBuf =
        std::env::temp_dir().join(format!("sandbox-pty-durable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (app, new_key) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    let key = new_key.expect("the first run creates a key");
    let session = create(&app, &key).await;

    let auth_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = auth_listener.local_addr().unwrap();
    let serve_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(auth_listener, serve_app).await.unwrap();
    });

    type_line(addr, &session, &key, "echo kept > kept.txt").await;
    server.abort();
    drop(app);

    // Restart on the same directory. The file the shell wrote must still be there.
    let (app, none) = open_durable(Arc::new(Scheduler::new()), &dir)
        .await
        .unwrap();
    assert!(none.is_none());
    let (status, body) = call(
        &app,
        &key,
        "GET",
        &format!("/v1/sessions/{session}/fs/kept.txt"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"kept\n");

    // The trace records the line the shell ran.
    let (status, body) = call(
        &app,
        &key,
        "GET",
        &format!("/v1/sessions/{session}/trace"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8_lossy(&body).contains("echo kept > kept.txt"),
        "the trace names the typed line: {}",
        String::from_utf8_lossy(&body)
    );
}
