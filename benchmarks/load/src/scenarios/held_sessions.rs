use crate::reporter::{LatencyHistogram, ScenarioResult};
use crate::scenarios::{
    create_exec_request, exec_outcome, new_session, write_file, ScenarioConfig,
};
use anyhow::Result;
use sandbox_session::{IsolateBackend, Session};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{ProcessesToUpdate, System};
use tokio::time::sleep;

/// Pause between one session's commands, in milliseconds. An agent waits on a model between
/// commands, so a closed loop with no pause would measure the shell, not many live sessions.
/// `HOLD_THINK_MS` overrides it.
const DEFAULT_THINK_MS: u64 = 100;

pub struct HeldSessionsScenario {
    config: ScenarioConfig,
}

/// Resident set size of this process, in bytes.
fn rss_bytes(system: &mut System) -> u64 {
    let Ok(pid) = sysinfo::get_current_pid() else {
        return 0;
    };
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map_or(0, |p| p.memory())
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

impl HeldSessionsScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Opens `concurrency` sessions, spread over the ramp-up, and keeps all of them live.
    /// Then, for the duration, every session runs `echo ready` once per think time. At the
    /// end the sessions are dropped. Create latency and exec latency are reported apart, and
    /// the process RSS is sampled before, while all sessions are held, and after the drop.
    pub async fn run(self) -> Result<ScenarioResult> {
        let n = self.config.concurrency;
        let think = Duration::from_millis(env_u64("HOLD_THINK_MS", DEFAULT_THINK_MS));
        let file_kib = env_u64("HOLD_FILE_KIB", 0) as usize;
        let mut system = System::new().map_err(|e| anyhow::anyhow!("sysinfo: {e}"))?;
        let rss_before = rss_bytes(&mut system);

        // Phase 1: create the sessions.
        let created_at = Instant::now();
        let ramp_step = if n > 0 {
            self.config.ramp_up / n as u32
        } else {
            Duration::ZERO
        };
        let mut creating = Vec::with_capacity(n);
        for i in 0..n {
            creating.push(tokio::spawn(async move {
                let delay = ramp_step * i as u32;
                if delay > Duration::ZERO {
                    sleep(delay).await;
                }
                let start = Instant::now();
                let session = new_session().await.map_err(|e| e.to_string())?;
                if file_kib > 0 {
                    // Each session gets different bytes, so the shared content-addressed
                    // store cannot deduplicate them.
                    let bytes = vec![(i % 251) as u8 ^ 0x5a; file_kib * 1024];
                    write_file(session.fs(), "/data.bin", &bytes)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok::<_, String>((session, start.elapsed()))
            }));
        }
        let mut sessions: Vec<Arc<IsolateBackend>> = Vec::with_capacity(n);
        let mut create_ns: Vec<u64> = Vec::with_capacity(n);
        let mut errors = 0usize;
        for handle in creating {
            match handle.await? {
                Ok((session, took)) => {
                    sessions.push(Arc::new(session));
                    create_ns.push(took.as_nanos() as u64);
                }
                Err(e) => {
                    errors += 1;
                    tracing::debug!(error = %e, "create failed");
                }
            }
        }
        let all_created = created_at.elapsed();
        let live = sessions.len();
        let rss_held = rss_bytes(&mut system);
        println!(
            "held_sessions: {live}/{n} sessions live after {:.2}s ({} create errors)",
            all_created.as_secs_f64(),
            errors
        );
        println!(
            "held_sessions: RSS before {:.1} MiB, with {live} sessions {:.1} MiB, per session {:.1} KiB",
            mib(rss_before),
            mib(rss_held),
            rss_held.saturating_sub(rss_before) as f64 / 1024.0 / live.max(1) as f64
        );

        // Phase 2: drive every live session for the duration.
        let exec_start = Instant::now();
        let deadline = exec_start + self.config.duration;
        let mut driving = Vec::with_capacity(live);
        for (i, session) in sessions.iter().cloned().enumerate() {
            driving.push(tokio::spawn(async move {
                // Spread the first command over one think time, so the sessions do not
                // all fire in the same instant.
                sleep(think.mul_f64(i as f64 / live.max(1) as f64)).await;
                let mut samples: Vec<u64> = Vec::new();
                let mut failed = 0usize;
                while Instant::now() < deadline {
                    let start = Instant::now();
                    let (ok, error) =
                        exec_outcome(session.exec(create_exec_request("echo ready")).await);
                    samples.push(start.elapsed().as_nanos() as u64);
                    if !ok {
                        failed += 1;
                        tracing::debug!(error = error.as_deref().unwrap_or(""), "exec failed");
                    }
                    sleep(think).await;
                }
                (samples, failed)
            }));
        }
        let mut exec_ns: Vec<u64> = Vec::new();
        let mut exec_errors = 0usize;
        for handle in driving {
            let (samples, failed) = handle.await?;
            exec_ns.extend(samples);
            exec_errors += failed;
        }
        let exec_elapsed = exec_start.elapsed();
        let rss_after_exec = rss_bytes(&mut system);
        println!(
            "held_sessions: RSS after the exec phase {:.1} MiB",
            mib(rss_after_exec)
        );

        // Phase 3: drop the sessions.
        for session in sessions {
            if let Ok(session) = Arc::try_unwrap(session) {
                let _ = Box::new(session).destroy().await;
            }
        }
        let rss_after = rss_bytes(&mut system);
        println!(
            "held_sessions: RSS after the drop {:.1} MiB",
            mib(rss_after)
        );

        let total_errors = errors + exec_errors;
        let total_iterations = n + exec_ns.len();
        let mut latencies = HashMap::new();
        latencies.insert(
            "create".to_string(),
            LatencyHistogram::from_samples(&create_ns),
        );
        latencies.insert("exec".to_string(), LatencyHistogram::from_samples(&exec_ns));
        // The summary prints "overall", so it carries the exec latency under all sessions.
        latencies.insert(
            "overall".to_string(),
            LatencyHistogram::from_samples(&exec_ns),
        );
        let mut throughput = HashMap::new();
        throughput.insert(
            "overall".to_string(),
            exec_ns.len() as f64 / exec_elapsed.as_secs_f64(),
        );
        throughput.insert(
            "create".to_string(),
            live as f64 / all_created.as_secs_f64().max(f64::EPSILON),
        );
        let mut error_rates = HashMap::new();
        error_rates.insert(
            "overall".to_string(),
            total_errors as f64 / total_iterations.max(1) as f64,
        );

        Ok(ScenarioResult {
            scenario: "held_sessions".to_string(),
            duration_secs: all_created.as_secs_f64() + exec_elapsed.as_secs_f64(),
            concurrency: live,
            total_iterations,
            total_errors,
            latencies,
            throughput,
            error_rates,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }
}
