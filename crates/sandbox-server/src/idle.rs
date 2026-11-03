//! Idle sleep (PLAN §4.14, ADR 0015). A session that no request has touched for a
//! while is put to sleep, so it holds no memory. The next request wakes it.
//!
//! A sweep never waits for a session. A session whose gate is held, because a
//! command or a file operation is running on it, is skipped until the next sweep.

use crate::api::{now_millis, sleep_locked, AppState};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Puts to sleep every session that no request has touched for `after`. Returns
/// how many went to sleep. A session busy with a request is skipped, not waited
/// for.
pub async fn sweep(state: &AppState, after: Duration) -> usize {
    let cutoff = now_millis().saturating_sub(u64::try_from(after.as_millis()).unwrap_or(u64::MAX));
    let candidates: Vec<_> = {
        let table = state.sessions.0.read().await;
        table
            .iter()
            .filter(|(_, entry)| {
                entry.session.is_some() && entry.last_active.load(Ordering::Relaxed) <= cutoff
            })
            .map(|(id, entry)| (*id, entry.gate.clone()))
            .collect()
    };

    let mut slept = 0;
    for (id, gate) in candidates {
        // A session that is busy with a command is skipped, not waited for.
        if let Ok(_gate) = gate.try_lock_owned() {
            // A request may have touched the session since the scan. Check again
            // under the gate, so a session that was just used is not put to sleep.
            let still_idle = {
                let table = state.sessions.0.read().await;
                table.get(&id).is_some_and(|entry| {
                    entry.session.is_some() && entry.last_active.load(Ordering::Relaxed) <= cutoff
                })
            };
            if still_idle {
                match sleep_locked(state, id).await {
                    Ok(_) => slept += 1,
                    Err(error) => tracing::warn!(session = %id, ?error, "idle sleep failed"),
                }
            }
        }
    }
    slept
}

/// Runs `sweep` every `every`, for the life of the process. Returns the task to
/// abort when the schedule should stop.
pub fn schedule(state: AppState, after: Duration, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        loop {
            ticker.tick().await;
            let slept = sweep(&state, after).await;
            if slept > 0 {
                tracing::info!(slept, "idle sessions went to sleep");
            }
        }
    })
}
