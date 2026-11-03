//! The clustered durable state on Postgres (PLAN.md §4.8, M5).
//!
//! Several server nodes can share one database. The whole saved state is one row,
//! `sandbox_state`, holding the `Saved` document as JSON. Each save writes the
//! complete document, which is simple and easy to check: no log to replay, and
//! nothing to merge. A `revision` column makes each write conditional on the
//! revision this node last read or wrote. If another node wrote in between, the
//! save fails with `InvalidState`, and the caller reloads. Two nodes can never
//! overwrite each other silently.
//!
//! This stores the metadata: keys, sessions with their checkpoints, and snapshots.
//! The blob store (file contents) is not in the database. Every node that serves
//! these sessions must reach the same blob directory, such as a shared volume.
//! Moving blobs out of the file system is a separate change.
//!
//! Full-document writes cost more than the file store's appends for a large state.
//! The file store stays the default for one node, and this is the choice when
//! nodes must share state.

use crate::persist::{Saved, VERSION};
use sandbox_types::{ErrorCode, SandboxError};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::Mutex;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS sandbox_state (\
    id SMALLINT PRIMARY KEY CHECK (id = 1), \
    revision BIGINT NOT NULL, \
    body JSONB NOT NULL)";

/// Connections kept open to the database. The saves are serialized, so a few suffice.
const MAX_CONNECTIONS: u32 = 4;

/// The shared state in Postgres. See the module comment.
pub struct PgStore {
    pool: PgPool,
    /// The revision this node last read or wrote. `0` means the row does not exist yet.
    /// Saves are serialized by this lock, so they run one at a time on this node.
    revision: Mutex<i64>,
}

impl PgStore {
    /// Connects, creates the table if needed, and reads the saved state. An empty
    /// database has an empty state.
    pub async fn connect(url: &str) -> Result<(Self, Saved), SandboxError> {
        let pool = PgPoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect(url)
            .await
            .map_err(|e| db_error("connecting to the state database", e))?;
        sqlx::query(SCHEMA)
            .execute(&pool)
            .await
            .map_err(|e| db_error("creating the state table", e))?;
        let row: Option<(i64, serde_json::Value)> =
            sqlx::query_as("SELECT revision, body FROM sandbox_state WHERE id = 1")
                .fetch_optional(&pool)
                .await
                .map_err(|e| db_error("reading the saved state", e))?;
        let (revision, saved) = match row {
            None => (0, empty_state()),
            Some((revision, body)) => {
                // The version is checked before the rest is read, so a state of another
                // version is refused as such, not as a malformed document.
                let version = body.get("version").and_then(serde_json::Value::as_u64);
                if version != Some(u64::from(VERSION)) {
                    let found = version.map_or("none".to_string(), |v| v.to_string());
                    return Err(SandboxError::new(
                        ErrorCode::InvalidState,
                        format!("saved state is version {found}, and this server reads version {VERSION}"),
                    ));
                }
                let saved: Saved = serde_json::from_value(body).map_err(|e| {
                    SandboxError::internal(format!("saved state is not valid JSON: {e}"))
                })?;
                (revision, saved)
            }
        };
        Ok((
            Self {
                pool,
                revision: Mutex::new(revision),
            },
            saved,
        ))
    }

    /// Captures the state with `capture` and writes it, if no other node has written
    /// since this node last did. Otherwise the save fails with `InvalidState`, and
    /// nothing is written.
    pub async fn save<F, Fut>(&self, capture: F) -> Result<(), SandboxError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Saved, SandboxError>>,
    {
        let mut revision = self.revision.lock().await;
        let saved = capture().await?;
        let body = serde_json::to_value(&saved)
            .map_err(|e| SandboxError::internal(format!("cannot encode the state: {e}")))?;
        let written = if *revision == 0 {
            sqlx::query(
                "INSERT INTO sandbox_state (id, revision, body) VALUES (1, 1, $1) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&body)
            .execute(&self.pool)
            .await
            .map_err(|e| db_error("writing the state", e))?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE sandbox_state SET revision = revision + 1, body = $1 \
                 WHERE id = 1 AND revision = $2",
            )
            .bind(&body)
            .bind(*revision)
            .execute(&self.pool)
            .await
            .map_err(|e| db_error("writing the state", e))?
            .rows_affected()
        };
        if written != 1 {
            return Err(SandboxError::new(
                ErrorCode::InvalidState,
                "the saved state changed on another node; reload it and retry",
            ));
        }
        *revision = if *revision == 0 { 1 } else { *revision + 1 };
        Ok(())
    }
}

fn empty_state() -> Saved {
    Saved {
        version: VERSION,
        ..Saved::default()
    }
}

fn db_error(context: &str, error: sqlx::Error) -> SandboxError {
    SandboxError::new(ErrorCode::Unavailable, format!("{context}: {error}"))
}
