//! Unit tests for the HTTP middleware.

use sandbox_server::middleware::{fresh_context, TraceContext};

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
    assert!(TraceContext::parse(&format!("00-{}-{}-01", fresh.trace_id, fresh.span_id)).is_some());
}
