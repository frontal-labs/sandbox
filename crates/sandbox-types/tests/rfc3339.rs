use sandbox_types::rfc3339::{format_rfc3339, parse_rfc3339};
use std::time::Duration;

#[test]
fn known_dates_format_as_expected() {
    assert_eq!(format_rfc3339(Duration::ZERO), "1970-01-01T00:00:00.000Z");
    assert_eq!(
        format_rfc3339(Duration::from_millis(1_500)),
        "1970-01-01T00:00:01.500Z"
    );
    // 2000-03-01, the day after a leap day in a century year that is a leap year.
    assert_eq!(
        format_rfc3339(Duration::from_secs(951_868_800)),
        "2000-03-01T00:00:00.000Z"
    );
    // 2026-10-15T12:00:00Z
    assert_eq!(
        format_rfc3339(Duration::from_secs(1_792_065_600)),
        "2026-10-15T12:00:00.000Z"
    );
}

#[test]
fn round_trips_through_text() {
    for millis in [
        0u64,
        1,
        999,
        1_000,
        86_399_999,
        951_868_800_123,
        4_102_444_800_000,
    ] {
        let since = Duration::from_millis(millis);
        let text = format_rfc3339(since);
        assert_eq!(parse_rfc3339(&text).unwrap(), since, "{text}");
    }
}

#[test]
fn accepts_optional_and_long_fractions() {
    assert_eq!(
        parse_rfc3339("1970-01-01T00:00:01Z").unwrap(),
        Duration::from_secs(1)
    );
    assert_eq!(
        parse_rfc3339("1970-01-01T00:00:01.5Z").unwrap(),
        Duration::from_millis(1_500)
    );
    assert_eq!(
        parse_rfc3339("1970-01-01T00:00:01.123456789Z").unwrap(),
        Duration::from_millis(1_123)
    );
}

#[test]
fn rejects_other_forms() {
    for bad in [
        "",
        "1970-01-01T00:00:00",
        "1970-01-01 00:00:00Z",
        "1970-13-01T00:00:00Z",
        "1970-01-01T25:00:00Z",
        "1970-01-01T00:00:00+01:00",
        "1970-01-01T00:00:00.Z",
        "{\"secs_since_epoch\":0,\"nanos_since_epoch\":0}",
    ] {
        assert!(parse_rfc3339(bad).is_err(), "accepted {bad}");
    }
}
