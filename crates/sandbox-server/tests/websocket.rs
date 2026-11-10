//! The WebSocket exec route at `/v1/sessions/{id}/ws`, served over a real TCP socket.
//! The first client message is the exec request. The server then sends each event as
//! a text message and closes after the final `Exit` or `Error` (ADR 0030).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures::{SinkExt, StreamExt};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

/// Serves the router on an ephemeral port, and returns its address.
async fn serve(app: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn create_session(app: &axum::Router, key: &str) -> String {
    let config = serde_json::to_vec(
        &serde_json::to_value(SessionConfig {
            tenant_id: TenantId::nil(),
            tier_preference: TierPreference::Isolate,
            ..SessionConfig::default()
        })
        .unwrap(),
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(config))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    value["session_id"].as_str().unwrap().to_string()
}

/// Opens the socket for `session` with `key` as the bearer token.
#[allow(clippy::result_large_err)]
async fn connect(
    addr: SocketAddr,
    session: &str,
    key: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    connect_with_query(addr, session, key, "").await
}

/// Connects to the exec route with a query string, such as `stdin=live`.
async fn connect_with_query(
    addr: SocketAddr,
    session: &str,
    key: &str,
    query: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let url = format!("ws://{addr}/v1/sessions/{session}/ws{query}");
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {key}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(|error| error.to_string())
}

/// Reads every event until the server closes the connection.
async fn events_until_close(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<Value> {
    let mut events = Vec::new();
    while let Some(message) = socket.next().await {
        match message.unwrap() {
            Message::Text(text) => events.push(serde_json::from_str(&text).unwrap()),
            Message::Close(_) => break,
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    events
}

#[tokio::test]
async fn a_command_streams_its_output_then_exits() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    let request =
        json!({"argv": ["sh", "-c", "echo hi; echo oops >&2"], "env": {}, "stream": true});
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();
    let events = events_until_close(&mut socket).await;

    let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(kinds.last(), Some(&"exit"), "{events:?}");
    let exit = events.last().unwrap();
    assert_eq!(exit["exit_code"], 0);
    let stdout: Vec<u8> = events
        .iter()
        .filter(|e| e["type"] == "output" && e["stream"] == "stdout")
        .flat_map(|e| {
            e["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8)
        })
        .collect();
    assert_eq!(stdout, b"hi\n");
}

#[tokio::test]
async fn a_first_message_that_is_not_a_request_is_an_error() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    socket
        .send(Message::Text("not json".to_string().into()))
        .await
        .unwrap();
    let events = events_until_close(&mut socket).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["type"], "error");
    assert_eq!(events[0]["code"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn an_empty_argv_is_an_error_and_runs_nothing() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    socket
        .send(Message::Text(
            json!({"argv": [], "env": {}, "stream": true})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let events = events_until_close(&mut socket).await;
    assert_eq!(events[0]["code"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn the_upgrade_is_refused_without_a_valid_key_or_session() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let wrong_key = connect(addr, &session, "sbx_not_a_key").await;
    assert!(wrong_key.is_err(), "a bad key must not upgrade");
    let missing = connect(addr, "01ARZ3NDEKTSV4RRFFQ69G5FAV", &key).await;
    assert!(missing.is_err(), "an unknown session must not upgrade");
}

/// A command that prints once, then loops far longer than any test waits.
fn long_running_request() -> String {
    json!({
        "argv": ["sh", "-c", "echo start; i=0; while [ $i -lt 100000000 ]; do i=$((i+1)); done"],
        "env": {},
        "stream": true,
    })
    .to_string()
}

/// Reads until the command has printed, so that it is known to be running.
async fn until_first_output(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    while let Some(message) = socket.next().await {
        if let Message::Text(text) = message.unwrap() {
            let event: Value = serde_json::from_str(&text).unwrap();
            if event["type"] == "output" {
                return;
            }
        }
    }
    panic!("the connection closed before the command printed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_message_stops_the_command_with_a_cancelled_error() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    socket
        .send(Message::Text(long_running_request().into()))
        .await
        .unwrap();
    until_first_output(&mut socket).await;
    socket
        .send(Message::Text(json!({"type": "cancel"}).to_string().into()))
        .await
        .unwrap();

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        events_until_close(&mut socket),
    )
    .await
    .expect("a cancelled command must end promptly");
    let last = events.last().expect("the stream ends with an event");
    assert_eq!(last["type"], "error", "{events:?}");
    assert_eq!(last["code"], "CANCELLED", "{events:?}");
}

/// Each stopping signal ends the command the way a cancel does. A signal that is
/// not a stopping signal, such as `USR1`, leaves the command running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopping_signal_stops_the_command_with_a_cancelled_error() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    socket
        .send(Message::Text(long_running_request().into()))
        .await
        .unwrap();
    until_first_output(&mut socket).await;
    // A signal the server does not stop on is ignored, so the command keeps running.
    socket
        .send(Message::Text(
            json!({"type": "signal", "name": "USR1"}).to_string().into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type": "signal", "name": "INT"}).to_string().into(),
        ))
        .await
        .unwrap();

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        events_until_close(&mut socket),
    )
    .await
    .expect("a signalled command must end promptly");
    let last = events.last().expect("the stream ends with an event");
    assert_eq!(last["type"], "error", "{events:?}");
    assert_eq!(last["code"], "CANCELLED", "{events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_connection_cancels_the_command_and_frees_the_session() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app.clone()).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    socket
        .send(Message::Text(long_running_request().into()))
        .await
        .unwrap();
    until_first_output(&mut socket).await;
    drop(socket);

    // The session's gate is held by the running command. A REST exec on the same
    // session can only finish once the disconnect has cancelled that command.
    let exec = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri(format!("/v1/sessions/{session}/exec"))
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"argv": ["sh", "-c", "echo after"], "env": {}, "stream": false}).to_string(),
            ))
            .unwrap(),
    );
    let response = tokio::time::timeout(std::time::Duration::from_secs(20), exec)
        .await
        .expect("the disconnect must free the session")
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// The bytes of the stdout events, in order.
fn stdout_of(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .filter(|e| e["type"] == "output" && e["stream"] == "stdout")
        .flat_map(|e| {
            e["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_stdin_is_read_once_the_client_sends_eof() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect_with_query(addr, &session, &key, "?stdin=live")
        .await
        .unwrap();
    let request = json!({"argv": ["cat"], "env": {}, "stream": true});
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type": "stdin", "data": "hello\n"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type": "stdin", "data": "world\n"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type": "stdin_eof"}).to_string().into(),
        ))
        .await
        .unwrap();

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        events_until_close(&mut socket),
    )
    .await
    .expect("the reader must run once its input is complete");
    assert_eq!(events.last().unwrap()["exit_code"], 0, "{events:?}");
    assert_eq!(stdout_of(&events), b"hello\nworld\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_that_reads_no_input_starts_without_eof() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect_with_query(addr, &session, &key, "?stdin=live")
        .await
        .unwrap();
    let request = json!({"argv": ["echo", "hi"], "env": {}, "stream": true});
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        events_until_close(&mut socket),
    )
    .await
    .expect("echo must not wait for input it does not read");
    assert_eq!(events.last().unwrap()["exit_code"], 0, "{events:?}");
    assert_eq!(stdout_of(&events), b"hi\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_while_waiting_for_input_runs_nothing() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect_with_query(addr, &session, &key, "?stdin=live")
        .await
        .unwrap();
    let request = json!({"argv": ["cat"], "env": {}, "stream": true});
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type": "stdin", "data": "x"}).to_string().into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(json!({"type": "cancel"}).to_string().into()))
        .await
        .unwrap();

    let events = events_until_close(&mut socket).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["type"], "error", "{events:?}");
    assert_eq!(events[0]["code"], "CANCELLED", "{events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_live_query_a_reader_does_not_wait() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await.unwrap();
    let request = json!({"argv": ["cat"], "env": {}, "stream": true});
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        events_until_close(&mut socket),
    )
    .await
    .expect("without stdin=live the reader runs at once, with empty input");
    assert_eq!(events.last().unwrap()["exit_code"], 0, "{events:?}");
    assert!(stdout_of(&events).is_empty());
}
