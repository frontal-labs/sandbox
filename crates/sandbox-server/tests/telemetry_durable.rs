//! Request spans reach an OTLP collector with the caller's trace and parent. The collector is a
//! local HTTP server that keeps what it receives.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::Json;
use sandbox_scheduler::Scheduler;
use sandbox_server::otel::SpanExporter;
use sandbox_server::{open_durable_with_options, ServerOptions};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn a_durable_server_exports_its_request_spans() {
    let received: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    let collector = axum::Router::new().route(
        "/v1/traces",
        post(move |Json(body): Json<Value>| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(body);
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, collector).await.unwrap() });

    let dir = std::env::temp_dir().join(format!("sandbox-otel-durable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let exporter = SpanExporter::start(&format!("http://{addr}"));
    let (app, _state, _) = open_durable_with_options(
        Arc::new(Scheduler::new()),
        &dir,
        ServerOptions {
            telemetry: Some(exporter),
            ..ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let trace = "1af7651916cd43dd8448eb211c80319d";
    let parent = "c7ad6b7169203332";
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/openapi.json")
                .header(header::AUTHORIZATION, "Bearer not-a-real-key")
                .header("traceparent", format!("00-{trace}-{parent}-01"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut span = None;
    for _ in 0..50 {
        let found = received.lock().unwrap().iter().find_map(|batch| {
            batch["resourceSpans"][0]["scopeSpans"][0]["spans"]
                .as_array()
                .and_then(|spans| spans.iter().find(|s| s["traceId"] == trace).cloned())
        });
        if found.is_some() {
            span = found;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let span = span.expect("the collector received the request's span");
    assert_eq!(span["parentSpanId"], parent);
    assert_eq!(span["name"], "GET /v1/openapi.json");
    let _ = std::fs::remove_dir_all(&dir);
}
