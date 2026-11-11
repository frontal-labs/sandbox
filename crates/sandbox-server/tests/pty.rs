//! The interactive shell at `/v1/sessions/{id}/pty`, over a real TCP socket (PLAN §6, M6).
//! Covers starting a session, typed input and output, resize, Ctrl-C on a running command,
//! and the exit status of `exit` and Ctrl-D.

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
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
    serde_json::from_slice::<Value>(&bytes).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn connect(addr: SocketAddr, session: &str, key: &str) -> Socket {
    let url = format!("ws://{addr}/v1/sessions/{session}/pty");
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {key}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn send_input(socket: &mut Socket, data: &str) {
    socket
        .send(Message::Text(
            json!({"type": "input", "data": data}).to_string().into(),
        ))
        .await
        .unwrap();
}

/// Reads the shell's output until it contains `wanted`, and returns everything read so far.
/// Fails if the shell stays silent past the timeout.
async fn output_until(socket: &mut Socket, wanted: &str) -> String {
    let mut seen = String::new();
    let read = async {
        while let Some(message) = socket.next().await {
            match message.unwrap() {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "output" {
                        seen.push_str(event["data"].as_str().unwrap());
                        if seen.contains(wanted) {
                            return;
                        }
                    }
                }
                other => panic!("unexpected frame: {other:?}"),
            }
        }
        panic!("the shell closed before printing {wanted:?}; saw {seen:?}");
    };
    tokio::time::timeout(Duration::from_secs(20), read)
        .await
        .unwrap_or_else(|_| panic!("the shell did not print {wanted:?}"));
    seen
}

/// The exit frame and the close that follow it.
async fn exit_of(socket: &mut Socket) -> Value {
    let read = async {
        while let Some(message) = socket.next().await {
            match message.unwrap() {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "exit" {
                        return event;
                    }
                }
                Message::Close(_) => panic!("the shell closed without an exit frame"),
                _ => {}
            }
        }
        panic!("the connection ended without an exit frame");
    };
    tokio::time::timeout(Duration::from_secs(20), read)
        .await
        .expect("the shell reports its exit")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_starts_with_a_prompt() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_input_runs_and_its_output_comes_back() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "echo hello\r").await;
    output_until(&mut socket, "hello\r\n$ ").await;
    // State persists between lines.
    send_input(&mut socket, "name=kept\r").await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "echo $name\r").await;
    output_until(&mut socket, "kept\r\n$ ").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resize_is_visible_to_the_next_command() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    socket
        .send(Message::Text(
            json!({"type": "resize", "cols": 100, "rows": 30})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    send_input(&mut socket, "echo $COLUMNS $LINES\r").await;
    output_until(&mut socket, "100 30\r\n$ ").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_stops_a_running_command_and_the_prompt_returns() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    // A loop that never ends on its own.
    send_input(&mut socket, "i=0; while true; do i=$((i+1)); done\r").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    send_input(&mut socket, "\u{3}").await;
    output_until(&mut socket, "^C\r\n$ ").await;

    // The session still works after the interrupt.
    send_input(&mut socket, "echo after\r").await;
    output_until(&mut socket, "after\r\n$ ").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_ends_the_shell_with_its_status() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "exit 3\r").await;
    let exit = exit_of(&mut socket).await;
    assert_eq!(exit["code"], 3, "{exit}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_d_on_an_empty_line_ends_the_shell_with_zero() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "\u{4}").await;
    let exit = exit_of(&mut socket).await;
    assert_eq!(exit["code"], 0, "{exit}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_exit_reports_the_status_of_the_last_command() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "false\r").await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "exit\r").await;
    let exit = exit_of(&mut socket).await;
    assert_eq!(exit["code"], 1, "{exit}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_typed_line_reaches_a_running_cat_and_ctrl_d_ends_it() {
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let session = create_session(&app, &key).await;
    let addr = serve(app).await;

    let mut socket = connect(addr, &session, &key).await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "cat\r").await;
    // cat is running now, so it reads what is typed and prints it before the input ends.
    send_input(&mut socket, "hello\r").await;
    output_until(&mut socket, "hello\r\n").await;
    send_input(&mut socket, "again\r").await;
    output_until(&mut socket, "again\r\n").await;
    // Ctrl-D ends cat's input, so cat finishes and the prompt returns.
    send_input(&mut socket, "\u{4}").await;
    output_until(&mut socket, "$ ").await;
    send_input(&mut socket, "echo done\r").await;
    output_until(&mut socket, "done\r\n$ ").await;
}
