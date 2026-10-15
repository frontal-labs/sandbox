//! Serde for `SystemTime` as an RFC 3339 string in UTC with millisecond
//! precision, such as `2026-10-15T12:00:00.000Z`. The API documents timestamps
//! as `date-time`, so this is the wire form, not serde's default struct.

use serde::{de, Deserialize, Deserializer, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn serialize<S: Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    serializer.serialize_str(&format_rfc3339(since))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
    let text = String::deserialize(deserializer)?;
    let since = parse_rfc3339(&text).map_err(de::Error::custom)?;
    Ok(UNIX_EPOCH + since)
}

/// Formats `since` (time after the epoch) as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_rfc3339(since: Duration) -> String {
    let secs = since.as_secs() as i64;
    let millis = since.subsec_millis();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parses the form `format_rfc3339` writes. Fractional seconds are optional.
pub fn parse_rfc3339(text: &str) -> Result<Duration, String> {
    let bad = || format!("not an RFC 3339 UTC timestamp: {text}");
    let body = text.strip_suffix('Z').ok_or_else(bad)?;
    let (date, time) = body.split_once('T').ok_or_else(bad)?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let month: u32 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let day: u32 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(bad());
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.split(':');
    let hour: u64 = clock_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let minute: u64 = clock_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let second: u64 = clock_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return Err(bad());
    }
    let millis: u64 = match fraction {
        None => 0,
        Some(f) if !f.is_empty() && f.len() <= 9 && f.bytes().all(|b| b.is_ascii_digit()) => {
            // The first three digits are milliseconds; the rest are truncated.
            let padded = format!("{f:0<3}");
            padded[..3].parse().map_err(|_| bad())?
        }
        Some(_) => return Err(bad()),
    };
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Err(bad());
    }
    let secs = days as u64 * 86_400 + hour * 3600 + minute * 60 + second;
    Ok(Duration::from_secs(secs) + Duration::from_millis(millis))
}

/// Civil (proleptic Gregorian) date for days since 1970-01-01.
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Inverse of `civil_from_days`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
