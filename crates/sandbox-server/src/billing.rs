#![cfg_attr(not(feature = "clustered"), allow(dead_code))]
//! The billing event stream (PLAN §4.14, M5). Session starts and stops, and the usage reading
//! after each command, are appended to one table in the clustered Postgres database. The table's
//! sequence number is the position a consumer saves, so a consumer resumes from any point, and a
//! restart cannot lose an event the server had acknowledged: each event is written before the
//! request that caused it is answered.
//!
//! Only a clustered server has the stream. On other builds `BillingLog` has no values, so
//! `AppState::billing` is always `None`, and the code that appends to it is never reached.

use crate::metering::SessionMeter;
use serde::{Deserialize, Serialize};

/// What happened, as the stream records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    #[serde(rename = "session.started")]
    SessionStarted,
    #[serde(rename = "session.stopped")]
    SessionStopped,
    #[serde(rename = "usage.reading")]
    UsageReading,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::SessionStarted => "session.started",
            EventKind::SessionStopped => "session.stopped",
            EventKind::UsageReading => "usage.reading",
        }
    }
}

/// One event before it is stored. `payload` holds the session's meter for the kinds that
/// report usage, and is empty for a start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingEvent {
    pub key_id: String,
    pub session_id: String,
    pub kind: EventKind,
    pub at_ms: u64,
    pub payload: serde_json::Value,
}

impl BillingEvent {
    pub fn session_started(key_id: &str, session_id: &str, at_ms: u64) -> Self {
        Self {
            key_id: key_id.to_string(),
            session_id: session_id.to_string(),
            kind: EventKind::SessionStarted,
            at_ms,
            payload: serde_json::json!({}),
        }
    }

    pub fn session_stopped(
        key_id: &str,
        session_id: &str,
        at_ms: u64,
        meter: &SessionMeter,
    ) -> Self {
        Self {
            key_id: key_id.to_string(),
            session_id: session_id.to_string(),
            kind: EventKind::SessionStopped,
            at_ms,
            payload: serde_json::to_value(meter).unwrap_or_default(),
        }
    }

    pub fn usage_reading(key_id: &str, session_id: &str, at_ms: u64, meter: &SessionMeter) -> Self {
        Self {
            key_id: key_id.to_string(),
            session_id: session_id.to_string(),
            kind: EventKind::UsageReading,
            at_ms,
            payload: serde_json::to_value(meter).unwrap_or_default(),
        }
    }
}

/// An event as read back, with its position in the stream.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub event: BillingEvent,
}

#[cfg(feature = "clustered")]
mod postgres {
    use super::{BillingEvent, EventKind, StoredEvent};
    use crate::metering::SessionMeter;
    use sandbox_types::SandboxError;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;

    const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS billing_events (\
        seq BIGSERIAL PRIMARY KEY, \
        key_id TEXT NOT NULL, \
        session_id TEXT NOT NULL, \
        kind TEXT NOT NULL, \
        at_ms BIGINT NOT NULL, \
        payload JSONB NOT NULL)";

    const MAX_CONNECTIONS: u32 = 4;

    /// The billing stream in the shared database.
    #[derive(Clone)]
    pub struct BillingLog {
        pool: PgPool,
    }

    fn internal(what: &str, e: sqlx::Error) -> SandboxError {
        SandboxError::internal(format!("billing stream: {what}: {e}"))
    }

    impl BillingLog {
        /// Connects to the database and creates the table if it is missing.
        pub async fn connect(url: &str) -> Result<Self, SandboxError> {
            let pool = PgPoolOptions::new()
                .max_connections(MAX_CONNECTIONS)
                .connect(url)
                .await
                .map_err(|e| internal("connect", e))?;
            sqlx::query(SCHEMA)
                .execute(&pool)
                .await
                .map_err(|e| internal("create the table", e))?;
            Ok(Self { pool })
        }

        /// Appends one event and returns its position. The write is committed before this
        /// returns, so a caller that answers after it has made the event durable.
        pub async fn append(&self, event: &BillingEvent) -> Result<u64, SandboxError> {
            let (seq,): (i64,) = sqlx::query_as(
                "INSERT INTO billing_events (key_id, session_id, kind, at_ms, payload) \
                 VALUES ($1, $2, $3, $4, $5) RETURNING seq",
            )
            .bind(&event.key_id)
            .bind(&event.session_id)
            .bind(event.kind.as_str())
            .bind(event.at_ms as i64)
            .bind(&event.payload)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| internal("append", e))?;
            u64::try_from(seq)
                .map_err(|_| SandboxError::internal("billing stream: negative position"))
        }

        /// The events of one key after `after`, oldest first, at most `limit`. A consumer saves
        /// the last `seq` it handled and passes it back as `after`.
        pub async fn read_after(
            &self,
            key_id: &str,
            after: u64,
            limit: i64,
        ) -> Result<Vec<StoredEvent>, SandboxError> {
            let rows: Vec<(i64, String, String, String, i64, serde_json::Value)> = sqlx::query_as(
                "SELECT seq, key_id, session_id, kind, at_ms, payload FROM billing_events \
                 WHERE key_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
            )
            .bind(key_id)
            .bind(after as i64)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| internal("read", e))?;
            rows.into_iter()
                .map(|(seq, key_id, session_id, kind, at_ms, payload)| {
                    let kind = kind_from_str(&kind)?;
                    Ok(StoredEvent {
                        seq: seq as u64,
                        event: BillingEvent {
                            key_id,
                            session_id,
                            kind,
                            at_ms: at_ms as u64,
                            payload,
                        },
                    })
                })
                .collect()
        }

        /// Every session that started, with its latest cumulative meter and whether it is still
        /// open, for restoring meters after a restart. A session with no reading (one that never
        /// ran a command) comes back with a zero meter, so it is still reported.
        pub async fn latest_meters(
            &self,
        ) -> Result<Vec<(String, String, SessionMeter, bool)>, sandbox_types::SandboxError>
        {
            let rows: Vec<(String, String, serde_json::Value, bool)> = sqlx::query_as(
                "SELECT s.key_id, s.session_id, COALESCE(r.payload, '{}'::jsonb), \
                        NOT EXISTS (SELECT 1 FROM billing_events x \
                                    WHERE x.session_id = s.session_id AND x.kind = 'session.stopped') \
                 FROM (SELECT DISTINCT key_id, session_id FROM billing_events \
                       WHERE kind = 'session.started') s \
                 LEFT JOIN LATERAL (SELECT payload FROM billing_events e \
                                    WHERE e.session_id = s.session_id AND e.kind = 'usage.reading' \
                                    ORDER BY e.seq DESC LIMIT 1) r ON true \
                 ORDER BY s.key_id, s.session_id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| internal("restore", e))?;
            rows.into_iter()
                .map(|(key, session, payload, open)| {
                    let meter = serde_json::from_value(payload).map_err(|e| {
                        SandboxError::internal(format!("billing stream: bad reading: {e}"))
                    })?;
                    Ok((key, session, meter, open))
                })
                .collect()
        }
    }

    fn kind_from_str(kind: &str) -> Result<EventKind, SandboxError> {
        match kind {
            "session.started" => Ok(EventKind::SessionStarted),
            "session.stopped" => Ok(EventKind::SessionStopped),
            "usage.reading" => Ok(EventKind::UsageReading),
            other => Err(SandboxError::internal(format!(
                "billing stream: unknown kind {other}"
            ))),
        }
    }
}

#[cfg(feature = "clustered")]
pub use postgres::BillingLog;

/// Without the clustered build there is no stream, and this type has no values.
#[cfg(not(feature = "clustered"))]
#[derive(Clone)]
pub enum BillingLog {}

#[cfg(not(feature = "clustered"))]
impl BillingLog {
    pub async fn append(&self, _event: &BillingEvent) -> Result<u64, sandbox_types::SandboxError> {
        match *self {}
    }

    pub async fn read_after(
        &self,
        _key_id: &str,
        _after: u64,
        _limit: i64,
    ) -> Result<Vec<StoredEvent>, sandbox_types::SandboxError> {
        match *self {}
    }

    pub async fn latest_meters(
        &self,
    ) -> Result<Vec<(String, String, SessionMeter, bool)>, sandbox_types::SandboxError> {
        match *self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
