//! `SystemTime` to and from `google.protobuf.Timestamp`, including pre-epoch values.

use super::invalid;
use prost_types::Timestamp;
use sandbox_types::SandboxError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn to_timestamp(time: SystemTime) -> Timestamp {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => Timestamp {
            seconds: i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
            nanos: after.subsec_nanos() as i32,
        },
        Err(before) => {
            let back = before.duration();
            if back.subsec_nanos() == 0 {
                Timestamp {
                    seconds: -i64::try_from(back.as_secs()).unwrap_or(i64::MAX),
                    nanos: 0,
                }
            } else {
                Timestamp {
                    seconds: -i64::try_from(back.as_secs()).unwrap_or(i64::MAX) - 1,
                    nanos: (1_000_000_000 - back.subsec_nanos()) as i32,
                }
            }
        }
    }
}

pub fn from_timestamp(ts: &Timestamp) -> Result<SystemTime, SandboxError> {
    if !(0..1_000_000_000).contains(&ts.nanos) {
        return Err(invalid(format!(
            "timestamp nanos out of range: {}",
            ts.nanos
        )));
    }
    let nanos = Duration::from_nanos(u64::try_from(ts.nanos).unwrap_or(0));
    if ts.seconds >= 0 {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(ts.seconds as u64))
            .and_then(|t| t.checked_add(nanos))
            .ok_or_else(|| invalid("timestamp out of range"))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(ts.seconds.unsigned_abs()))
            .and_then(|t| t.checked_add(nanos))
            .ok_or_else(|| invalid("timestamp out of range"))
    }
}
