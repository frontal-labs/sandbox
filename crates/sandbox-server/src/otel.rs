//! Span export over OTLP/HTTP (JSON). Each request is one server span, with the trace and parent
//! IDs the request carried in `traceparent`, so a collector can join the server's spans to its
//! callers'. Spans are batched and sent from a background task. A failed export is logged and
//! dropped: tracing must never slow or fail a request.
//!
//! Configure a collector with `serve --otel-endpoint URL` (or `OTEL_EXPORTER_OTLP_ENDPOINT`).

use std::time::Duration;
use tokio::sync::mpsc;

/// One finished request, as the collector needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub start_unix_nanos: u128,
    pub end_unix_nanos: u128,
}

/// Sends spans to a collector. Cloning it is cheap, and every clone feeds the same task.
#[derive(Clone)]
pub struct SpanExporter {
    tx: mpsc::Sender<Span>,
}

const BATCH: usize = 64;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
/// Spans waiting to be sent. Past this, new spans are dropped, so memory stays bounded when the
/// collector is down.
const QUEUE: usize = 4096;

impl SpanExporter {
    /// Starts the export task for `endpoint`, such as `http://127.0.0.1:4318`.
    pub fn start(endpoint: &str) -> SpanExporter {
        let (tx, rx) = mpsc::channel(QUEUE);
        let url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
        tokio::spawn(run(url, rx));
        SpanExporter { tx }
    }

    /// Queues a span. Returns without waiting: a full queue drops the span.
    pub fn record(&self, span: Span) {
        let _ = self.tx.try_send(span);
    }
}

async fn run(url: String, mut rx: mpsc::Receiver<Span>) {
    let client = reqwest::Client::new();
    let mut batch: Vec<Span> = Vec::new();
    let mut ticker = tokio::time::interval(FLUSH_EVERY);
    loop {
        tokio::select! {
            span = rx.recv() => match span {
                Some(span) => {
                    batch.push(span);
                    if batch.len() >= BATCH {
                        send(&client, &url, &mut batch).await;
                    }
                }
                None => {
                    send(&client, &url, &mut batch).await;
                    return;
                }
            },
            _ = ticker.tick() => send(&client, &url, &mut batch).await,
        }
    }
}

async fn send(client: &reqwest::Client, url: &str, batch: &mut Vec<Span>) {
    if batch.is_empty() {
        return;
    }
    let body = otlp_json(batch).to_string();
    batch.clear();
    let result = client
        .post(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), "the span collector refused a batch")
        }
        Err(error) => tracing::warn!(%error, "a span batch could not be sent"),
    }
}

/// The OTLP/HTTP JSON body for a batch of server spans.
pub fn otlp_json(spans: &[Span]) -> serde_json::Value {
    use serde_json::json;
    let spans: Vec<serde_json::Value> = spans
        .iter()
        .map(|span| {
            let mut value = json!({
                "traceId": span.trace_id,
                "spanId": span.span_id,
                "name": span.name,
                "kind": 2,
                "startTimeUnixNano": span.start_unix_nanos.to_string(),
                "endTimeUnixNano": span.end_unix_nanos.to_string(),
                "attributes": [
                    {"key": "http.request.method", "value": {"stringValue": span.method}},
                    {"key": "url.path", "value": {"stringValue": span.path}},
                    {"key": "http.response.status_code", "value": {"intValue": span.status.to_string()}},
                ],
                "status": {"code": if span.status >= 500 { 2 } else { 1 }},
            });
            if let Some(parent) = &span.parent_span_id {
                value["parentSpanId"] = json!(parent);
            }
            value
        })
        .collect();
    json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": "frontal-sandbox"}},
            ]},
            "scopeSpans": [{
                "scope": {"name": "sandbox-server"},
                "spans": spans,
            }],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_is_otlp_json_with_the_trace_and_parent() {
        let span = Span {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "1111111111111111".into(),
            parent_span_id: Some("b7ad6b7169203331".into()),
            name: "GET /v1/usage".into(),
            method: "GET".into(),
            path: "/v1/usage".into(),
            status: 500,
            start_unix_nanos: 1,
            end_unix_nanos: 2,
        };
        let body = otlp_json(&[span]);
        let s = &body["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(s["traceId"], "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(s["parentSpanId"], "b7ad6b7169203331");
        assert_eq!(s["status"]["code"], 2);
        assert_eq!(s["startTimeUnixNano"], "1");
    }
}
