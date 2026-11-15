use sandbox_server::otel::{otlp_json, Span};

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
