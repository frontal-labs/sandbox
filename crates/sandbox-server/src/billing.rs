//! The billing event stream (PLAN §4.14, M5). Session starts and stops, and the usage reading
//! after each command, are appended to one stream. The stream's sequence number is the position
//! a consumer saves, so a consumer resumes from any point, and a restart cannot lose an event
//! the server had acknowledged: each event is written before the request that caused it is
//! answered.
//!
//! A clustered server keeps the stream in Postgres. A durable single-node server keeps it in
//! `billing.jsonl` in its data directory. Without either, `BillingLog` has no values, so
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
    #[cfg_attr(not(feature = "clustered"), allow(dead_code))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// The billing stream in a local JSONL file, for a durable single-node server.
/// Events are appended with a monotonically increasing sequence number, and the file is
/// replayed on startup to rebuild the meters before the server accepts requests.
#[cfg(not(feature = "clustered"))]
#[derive(Clone)]
pub struct BillingLog {
    path: std::path::PathBuf,
    next_seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(not(feature = "clustered"))]
impl BillingLog {
    /// Opens the file, replaying any existing events to determine the next sequence number.
    pub async fn open(path: std::path::PathBuf) -> Result<Self, sandbox_types::SandboxError> {
        use sandbox_types::SandboxError;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| SandboxError::internal(format!("billing stream: {e}")))?;
        }
        let next_seq = Self::read_all(&path)
            .await
            .map(|events| events.iter().map(|e| e.seq).max().map_or(0, |max| max + 1))
            .unwrap_or(0);
        Self::append_bytes(&path, b"").await?;
        Ok(Self {
            path,
            next_seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(next_seq)),
        })
    }

    /// Appends one event and returns its position. The write is synced before this returns.
    pub async fn append(&self, event: &BillingEvent) -> Result<u64, sandbox_types::SandboxError> {
        use sandbox_types::SandboxError;
        let seq = self
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let stored = StoredEvent {
            seq,
            event: event.clone(),
        };
        let mut bytes = serde_json::to_vec(&stored)
            .map_err(|e| SandboxError::internal(format!("billing stream: encode: {e}")))?;
        bytes.push(b'\n');
        Self::append_bytes(&self.path, &bytes).await?;
        Ok(seq)
    }

    /// Reads events for one key, skipping those at or before `after`, capped at `limit`.
    pub async fn read_after(
        &self,
        key_id: &str,
        after: u64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, sandbox_types::SandboxError> {
        let events = Self::read_all(&self.path).await?;
        Ok(events
            .into_iter()
            .filter(|e| e.event.key_id == key_id && e.seq > after)
            .take(limit as usize)
            .collect())
    }

    /// Reads the whole stream, returning all stored events in file order.
    async fn read_all(
        path: &std::path::Path,
    ) -> Result<Vec<StoredEvent>, sandbox_types::SandboxError> {
        use sandbox_types::SandboxError;
        let text = match tokio::fs::read_to_string(path).await {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(SandboxError::internal(format!(
                    "billing stream: cannot read {}: {e}",
                    path.display()
                )))
            }
        };
        Ok(text
            .lines()
            .filter_map(|line| serde_json::from_str::<StoredEvent>(line).ok())
            .collect::<Vec<_>>())
    }

    /// Appends `bytes` to the stream file, syncing it to disk before returning.
    async fn append_bytes(
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Result<(), sandbox_types::SandboxError> {
        crate::persist::append_synced(path, bytes).await
    }

    /// Replays the stream to rebuild per-session meters: the latest reading per session,
    /// and whether it is still open. Sessions with no reading come back at zero.
    pub async fn latest_meters(
        &self,
    ) -> Result<Vec<(String, String, SessionMeter, bool)>, sandbox_types::SandboxError> {
        let events = Self::read_all(&self.path).await?;
        let mut meters: std::collections::HashMap<String, (String, String, SessionMeter, bool)> =
            std::collections::HashMap::new();
        for stored in &events {
            let entry = meters
                .entry(stored.event.session_id.clone())
                .or_insert_with(|| {
                    (
                        stored.event.key_id.clone(),
                        stored.event.session_id.clone(),
                        SessionMeter::default(),
                        true,
                    )
                });
            match stored.event.kind {
                EventKind::SessionStarted => entry.3 = true,
                EventKind::UsageReading => {
                    if let Ok(meter) =
                        serde_json::from_value::<SessionMeter>(stored.event.payload.clone())
                    {
                        entry.2 = meter;
                    }
                }
                EventKind::SessionStopped => {
                    entry.3 = false;
                    if let Ok(meter) =
                        serde_json::from_value::<SessionMeter>(stored.event.payload.clone())
                    {
                        entry.2 = meter;
                    }
                }
            }
        }
        Ok(meters.into_values().collect())
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
