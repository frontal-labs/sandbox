//! Garbage collection of the blob store (PLAN §4.2, ADR 0013).
//!
//! The collector marks every blob a live session, a durable checkpoint, or a
//! snapshot can reach, then sweeps the rest. Commands and filesystem operations
//! hold a session's gate, so the collector holds every gate while it marks and
//! sweeps. No session can write a blob in between, and a blob written after the
//! collector lists the store is never swept.

use crate::api::{ApiError, AppState, ErrorBody};
use crate::auth::KeyId;
use axum::{extract::State, http::StatusCode, Extension};
use sandbox_session::{blob_store, committed_root, vfs_error};
use sandbox_types::{SandboxError, SessionId};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// The least time between two collections the server runs. A key can call the
/// endpoint, and every collection holds every session's gate for a moment, so
/// the interval bounds how much one key can slow the server.
/// Runs a collection every `every`, for the life of the process (ADR 0013). The
/// first run waits one interval, so a server that has just started does not
/// collect at once. Failures are logged, and the schedule continues. The task
/// stops when the returned handle is aborted.
pub fn schedule(state: AppState, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        loop {
            ticker.tick().await;
            match collect_garbage(&state).await {
                Ok(report) => tracing::info!(?report, "scheduled garbage collection finished"),
                Err(error) => tracing::error!(%error, "scheduled garbage collection failed"),
            }
        }
    })
}

pub const MIN_GC_INTERVAL: Duration = Duration::from_secs(60);

/// What one collection did. Written to the log when the collection finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GcReport {
    /// Committed roots the collection marked from: live sessions, durable
    /// checkpoints, and snapshots.
    pub roots: usize,
    /// Blobs the store held when the collection started.
    pub blobs_listed: usize,
    /// Blobs no root reached, which were removed.
    pub blobs_removed: usize,
    /// Chunk files no live blob names, which were removed.
    pub chunks_removed: usize,
}

/// Marks and sweeps the blob store. Returns what was removed.
pub async fn collect_garbage(state: &AppState) -> Result<GcReport, SandboxError> {
    let store = blob_store();
    loop {
        // A session created or destroyed during a pass leaves its gates stale, so the
        // pass starts again.
        if let Some(report) = sweep_once(state, store.as_ref()).await? {
            return Ok(report);
        }
    }
}

/// One mark and sweep. Returns `None` when the sessions changed while the gates were
/// taken, so the caller starts again.
async fn sweep_once(
    state: &AppState,
    store: &dyn sandbox_cas::BlobStore,
) -> Result<Option<GcReport>, SandboxError> {
    // Blobs that exist before the sweep. Blobs written later are not in
    // these lists, so they are kept.
    let listed = store.list(None).await?;
    let listed_parts = store.list_parts().await?;

    let members: Vec<(SessionId, Arc<Mutex<()>>)> = {
        let table = state.sessions.0.read().await;
        let mut members: Vec<_> = table.iter().map(|(id, e)| (*id, e.gate.clone())).collect();
        members.sort_by_key(|(id, _)| *id);
        members
    };
    // Taking the gates one at a time, without the table lock, cannot
    // deadlock with a command, which holds a gate and then the table.
    let mut gates = Vec::with_capacity(members.len());
    for (_, gate) in &members {
        gates.push(gate.clone().lock_owned().await);
    }

    let table = state.sessions.0.write().await;
    let unchanged =
        table.len() == members.len() && members.iter().all(|(id, _)| table.contains_key(id));
    if !unchanged {
        // A session was created or destroyed since the gates were taken. Start
        // again, so that every session that can write is gated.
        return Ok(None);
    }

    let mut roots = Vec::new();
    let mut raw_blobs: Vec<sandbox_types::hash::ContentHash> = Vec::new();
    for entry in members.iter().filter_map(|(id, _)| table.get(id)) {
        if let Some(session) = &entry.session {
            roots.push(committed_root(session.as_ref()).await?);
        }
        if let Some(checkpoint) = &entry.checkpoint {
            roots.push(checkpoint.root);
        }
        // Pinned checkpoints keep their blobs alive, so undo still has them.
        roots.extend(entry.roots.iter().map(|pinned| pinned.root));
        // Spilled outputs are plain blobs, not trees, so they are marked directly.
        raw_blobs.extend(
            entry
                .spilled
                .iter()
                .flat_map(|run| [run.stdout, run.stderr]),
        );
    }
    // A parked session is not in the table, but its record still pins roots and blobs.
    if let Some(parked) = &state.parked {
        for id in parked.ids().await? {
            if let Some(record) = parked.get(id).await? {
                roots.push(record.root);
                roots.extend(record.roots.iter().map(|pinned| pinned.root));
                raw_blobs.extend(
                    record
                        .spilled
                        .iter()
                        .flat_map(|run| [run.stdout, run.stderr]),
                );
            }
        }
    }
    {
        let records = state.snapshots.records.read().await;
        roots.extend(records.values().map(|record| record.root));
    }

    let mut live = sandbox_vfs::reachable_blobs(store, &roots)
        .await
        .map_err(vfs_error)?;
    live.extend(raw_blobs);
    let mut blobs_removed = 0;
    for hash in &listed {
        if !live.contains(hash) && store.delete(hash).await? {
            blobs_removed += 1;
        }
    }
    // A live blob may be stored as chunks. Chunks are shared, so only the
    // ones no live blob names are removed.
    let mut keep = HashSet::new();
    for hash in &live {
        keep.extend(store.parts(hash).await?);
    }
    let chunks_removed = store.delete_parts(&listed_parts, &keep).await?;

    Ok(Some(GcReport {
        roots: roots.len(),
        blobs_listed: listed.len(),
        blobs_removed,
        chunks_removed,
    }))
}

/// Start a garbage collection of the blob store. It runs in the background, so
/// the request does not wait for it (PLAN §4.2). The report is written to the
/// log when it finishes.
///
/// Any key may call it. A collection holds every session's gate for a moment,
/// so the server allows one per minute, and a call inside that window gets
/// `RESOURCE_EXHAUSTED`.
#[utoipa::path(
    post,
    path = "/v1/gc",
    tag = "admin",
    responses(
        (status = 202, description = "Collection started in the background"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 429, description = "A collection started less than a minute ago", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn run_gc(
    State(state): State<AppState>,
    Extension(_owner): Extension<KeyId>,
) -> Result<StatusCode, ApiError> {
    {
        let mut last = state.gc_last_run.lock().await;
        if let Some(at) = *last {
            let elapsed = at.elapsed();
            if elapsed < MIN_GC_INTERVAL {
                let wait = (MIN_GC_INTERVAL - elapsed).as_secs() + 1;
                return Err(SandboxError::resource_exhausted(format!(
                    "garbage collection started recently; try again in {wait}s"
                ))
                .into());
            }
        }
        *last = Some(Instant::now());
    }
    tokio::spawn(async move {
        match collect_garbage(&state).await {
            Ok(report) => tracing::info!(?report, "garbage collection finished"),
            Err(error) => tracing::error!(%error, "garbage collection failed"),
        }
    });
    Ok(StatusCode::ACCEPTED)
}
