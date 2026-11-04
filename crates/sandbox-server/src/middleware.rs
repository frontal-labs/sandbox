//! Per-request correlation, logging, and W3C trace context. Every request gets an ID, which a
//! client may supply in `x-request-id` (if it is printable and at most 128 bytes) or which the
//! server generates. The ID is returned in the same header, and it is on every log line for the
//! request.
//!
//! Trace context follows the W3C `traceparent` header: a valid incoming trace ID is kept, so a
//! request can be followed from the caller into the server, and the response carries the trace
//! ID with a new span ID for this server hop. Without a valid header, a new trace starts here.
//! The API key is never logged: only the method, the path, the status, and the time.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tracing::Instrument;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
pub const TRACEPARENT_HEADER: HeaderName = HeaderName::from_static("traceparent");

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

/// A W3C trace context: a 16-byte trace ID and an 8-byte span ID, both lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
}

impl TraceContext {
    /// Reads a `traceparent` value (`version-traceid-parentid-flags`). Returns `None` for any
    /// value the W3C format does not allow: a wrong length, uppercase or non-hex digits, or
    /// the all-zero IDs the spec forbids.
    pub fn parse(header: &str) -> Option<TraceContext> {
        let mut parts = header.split('-');
        let (version, trace_id, parent_id, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || version.len() != 2 || flags.len() != 2 {
            return None;
        }
        if version == "ff" || !is_hex(version) || !is_hex(flags) {
            return None;
        }
        if trace_id.len() != 32 || !is_hex(trace_id) || is_zero(trace_id) {
            return None;
        }
        if parent_id.len() != 16 || !is_hex(parent_id) || is_zero(parent_id) {
            return None;
        }
        Some(TraceContext {
            trace_id: trace_id.to_string(),
            span_id: parent_id.to_string(),
        })
    }

    /// The header value for this server's span of the trace, sampled as the caller set it.
    fn header(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.span_id)
    }
}

fn is_hex(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_zero(text: &str) -> bool {
    text.bytes().all(|b| b == b'0')
}

/// A new trace ID and span ID, from random bytes. A trace ID is never all zeros.
fn fresh_context() -> TraceContext {
    let trace = uuid::Uuid::new_v4().simple().to_string();
    let span = format!("{:016x}", uuid::Uuid::new_v4().as_u128() as u64);
    let span = if is_zero(&span) {
        "0000000000000001".to_string()
    } else {
        span
    };
    TraceContext {
        trace_id: trace,
        span_id: span,
    }
}

/// Runs the request inside a span that carries its ID and trace, and logs the outcome.
pub async fn request_context(request: Request, next: Next) -> Response {
    let id = request_id(&request);
    let incoming = request
        .headers()
        .get(&TRACEPARENT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(TraceContext::parse);
    let context = match incoming {
        Some(parent) => TraceContext {
            trace_id: parent.trace_id,
            span_id: fresh_context().span_id,
        },
        None => fresh_context(),
    };
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let span = tracing::info_span!(
        "request",
        request_id = %id,
        trace_id = %context.trace_id,
        span_id = %context.span_id,
        method = %method,
        path = %path,
    );
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
    if let Ok(value) = HeaderValue::from_str(&context.header()) {
        response.headers_mut().insert(TRACEPARENT_HEADER, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_traceparent_is_read_and_others_are_refused() {
        let good = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let parsed = TraceContext::parse(good).unwrap();
        assert_eq!(parsed.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(parsed.span_id, "b7ad6b7169203331");
        for bad in [
            "00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01",
            "00-00000000000000000000000000000000-b7ad6b7169203331-01",
            "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01",
            "00-0af7651916cd43dd8448eb211c8031-b7ad6b7169203331-01",
            "ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-extra",
            "garbage",
        ] {
            assert!(TraceContext::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_fresh_context_is_well_formed() {
        let fresh = fresh_context();
        assert!(
            TraceContext::parse(&format!("00-{}-{}-01", fresh.trace_id, fresh.span_id)).is_some()
        );
    }
}
