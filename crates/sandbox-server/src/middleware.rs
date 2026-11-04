//! Per-request correlation and logging. Every request gets an ID, which a client may
//! supply in `x-request-id` (if it is printable and at most 128 bytes) or which the server
//! generates. The ID is returned in the same header, and it is on every log line for the
//! request. The API key is never logged: only the method, the path, the status, and the time.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tracing::Instrument;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

const MAX_CLIENT_ID_BYTES: usize = 128;

/// The client's ID if it is usable, otherwise a new one.
fn request_id(request: &Request) -> String {
    request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_CLIENT_ID_BYTES
                && value.bytes().all(|b| b.is_ascii_graphic())
        })
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Runs the request inside a span that carries its ID, and logs the outcome.
pub async fn request_context(request: Request, next: Next) -> Response {
    let id = request_id(&request);
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let span = tracing::info_span!("request", request_id = %id, method = %method, path = %path);
    let started = std::time::Instant::now();
    let mut response = next.run(request).instrument(span.clone()).await;
    span.in_scope(|| {
        tracing::info!(
            status = response.status().as_u16(),
            duration_ms = started.elapsed().as_millis() as u64,
            "request completed"
        );
    });
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}
