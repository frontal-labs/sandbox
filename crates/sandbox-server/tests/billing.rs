//! Unit tests moved out of src/billing.rs.

use sandbox_server::billing::{BillingEvent, EventKind, StoredEvent};
use sandbox_server::metering::SessionMeter;

#[test]
fn kinds_have_the_names_the_stream_stores() {
    assert_eq!(EventKind::SessionStarted.as_str(), "session.started");
    assert_eq!(EventKind::SessionStopped.as_str(), "session.stopped");
    assert_eq!(EventKind::UsageReading.as_str(), "usage.reading");
}

#[test]
fn a_start_has_an_empty_payload_and_a_reading_carries_the_meter() {
    let start = BillingEvent::session_started("key", "s1", 10);
    assert_eq!(start.kind, EventKind::SessionStarted);
    assert_eq!(start.payload, serde_json::json!({}));

    let mut meter = SessionMeter::default();
    meter.record_exec(4, 9, 100);
    let reading = BillingEvent::usage_reading("key", "s1", 20, &meter);
    assert_eq!(reading.kind, EventKind::UsageReading);
    let back: SessionMeter = serde_json::from_value(reading.payload).unwrap();
    assert_eq!(back, meter);
}

#[test]
fn a_stop_carries_the_final_meter() {
    let mut meter = SessionMeter::default();
    meter.record_exec(1, 2, 3);
    let stop = BillingEvent::session_stopped("key", "s1", 30, &meter);
    assert_eq!(stop.kind, EventKind::SessionStopped);
    assert_eq!(stop.payload["execs"], 1);
}

#[test]
fn a_stored_event_serialises_its_position_beside_its_fields() {
    let stored = StoredEvent {
        seq: 7,
        event: BillingEvent::session_started("key", "s1", 10),
    };
    let json = serde_json::to_value(&stored).unwrap();
    assert_eq!(json["seq"], 7);
    assert_eq!(json["kind"], "session.started");
    assert_eq!(json["session_id"], "s1");
}
