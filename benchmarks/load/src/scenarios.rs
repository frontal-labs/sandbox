use crate::reporter::{LatencyHistogram, ScenarioResult};
use sandbox_types::{ExecRequest, SessionConfig, TierPreference};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;

pub mod concurrent_sessions;
pub mod exec_throughput;
pub mod mixed_workload;
pub mod snapshot_fork;

use concurrent_sessions::ConcurrentSessionsScenario;
use exec_throughput::ExecThroughputScenario;
use mixed_workload::MixedWorkloadScenario;
use snapshot_fork::SnapshotForkScenario;

#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    pub concurrency: usize,
    pub duration: Duration,
    pub ramp_up: Duration,
    /// Reserved for server mode. The scenarios run in-process today.
    #[allow(dead_code)]
    pub server_url: Option<String>,
}

// Use an enum instead of trait object for dyn-compatibility
#[derive(Debug, Clone, Copy)]
pub enum ScenarioType {
    ConcurrentSessions,
    ExecThroughput,
    SnapshotFork,
    MixedWorkload,
}

impl ScenarioType {
    pub fn from_str(name: &str) -> Option<Self> {
        match name {
            "concurrent_sessions" => Some(Self::ConcurrentSessions),
            "exec_throughput" => Some(Self::ExecThroughput),
            "snapshot_fork" => Some(Self::SnapshotFork),
            "mixed_workload" => Some(Self::MixedWorkload),
            _ => None,
        }
    }

    pub async fn run(self, config: ScenarioConfig) -> anyhow::Result<ScenarioResult> {
        match self {
            Self::ConcurrentSessions => ConcurrentSessionsScenario::new(config).run().await,
            Self::ExecThroughput => ExecThroughputScenario::new(config).run().await,
            Self::SnapshotFork => SnapshotForkScenario::new(config).run().await,
            Self::MixedWorkload => MixedWorkloadScenario::new(config).run().await,
        }
    }
}

pub fn get_scenario(name: &str, _config: ScenarioConfig) -> anyhow::Result<ScenarioType> {
    ScenarioType::from_str(name).ok_or_else(|| anyhow::anyhow!("Unknown scenario: {}", name))
}

pub async fn run_with_ramp_up<F, Fut>(
    config: &ScenarioConfig,
    worker_fn: F,
) -> anyhow::Result<ScenarioResult>
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<WorkerResult>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let mut handles = Vec::new();
    let start_time = Instant::now();
    let end_time = start_time + config.duration;

    let ramp_up_interval = if config.ramp_up > Duration::ZERO {
        config.ramp_up / config.concurrency as u32
    } else {
        Duration::ZERO
    };

    // Wrap worker_fn in Arc to avoid requiring Clone on the function itself
    let worker_fn = Arc::new(worker_fn);

    for i in 0..config.concurrency {
        let sem = semaphore.clone();
        let worker_fn = worker_fn.clone();
        let worker_start = start_time + ramp_up_interval * i as u32;
        let worker_end = end_time;

        let handle = tokio::spawn(async move {
            let delay = worker_start.saturating_duration_since(Instant::now());
            if delay > Duration::ZERO {
                sleep(delay).await;
            }

            let _permit = sem.acquire().await.unwrap();
            let worker_start = Instant::now();

            let mut results = Vec::new();
            while Instant::now() < worker_end {
                // Each closure times the operation it measures and reports it.
                let result = worker_fn(i).await?;
                results.extend(result.iterations);
            }

            Ok::<_, anyhow::Error>(WorkerResult {
                worker_id: i,
                iterations: results,
                total_duration: worker_start.elapsed(),
            })
        });

        handles.push(handle);
    }

    let mut all_iterations = Vec::new();
    let mut total_errors = 0;
    let mut total_iterations = 0;

    for handle in handles {
        let result = handle.await??;
        tracing::debug!(
            worker = result.worker_id,
            elapsed = ?result.total_duration,
            "worker finished"
        );
        for iter in result.iterations.iter().filter(|i| !i.success) {
            tracing::debug!(
                error = iter.error.as_deref().unwrap_or(""),
                "iteration failed"
            );
        }
        total_iterations += result.iterations.len();
        total_errors += result.iterations.iter().filter(|i| !i.success).count();
        all_iterations.extend(result.iterations);
    }

    let total_duration = start_time.elapsed();

    // Calculate latency percentiles
    let mut histogram = LatencyHistogram::new();
    for iter in &all_iterations {
        histogram.record(iter.duration.as_nanos() as u64);
    }

    let mut latencies = HashMap::new();
    latencies.insert("overall".to_string(), histogram);

    // Calculate throughput
    let throughput = total_iterations as f64 / total_duration.as_secs_f64();
    let mut tp_map = HashMap::new();
    tp_map.insert("overall".to_string(), throughput);

    // Calculate error rate
    let error_rate = total_errors as f64 / total_iterations.max(1) as f64;
    let mut err_map = HashMap::new();
    err_map.insert("overall".to_string(), error_rate);

    Ok(ScenarioResult {
        scenario: "generic".to_string(),
        duration_secs: total_duration.as_secs_f64(),
        concurrency: config.concurrency,
        total_iterations,
        total_errors,
        latencies,
        throughput: tp_map,
        error_rates: err_map,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

#[derive(Debug)]
pub struct WorkerResult {
    pub worker_id: usize,
    pub iterations: Vec<WorkerIteration>,
    pub total_duration: Duration,
}

#[derive(Debug)]
pub struct WorkerIteration {
    pub duration: Duration,
    pub success: bool,
    pub error: Option<String>,
}

impl Default for WorkerIteration {
    fn default() -> Self {
        Self {
            duration: Duration::ZERO,
            success: true,
            error: None,
        }
    }
}

pub fn create_session_config() -> SessionConfig {
    SessionConfig::default().with_tier(TierPreference::Auto)
}

pub fn create_exec_request(cmd: &str) -> ExecRequest {
    ExecRequest {
        argv: cmd.split_whitespace().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
        cwd: None,
        stdin: None,
        // The isolate backend does not enforce exec timeouts yet, so none is set.
        timeout_ms: None,
        stream: false,
    }
}

/// Outcome of one exec: success means the command ran and exited 0.
pub fn exec_outcome(
    result: Result<sandbox_types::ExecResult, sandbox_types::SandboxError>,
) -> (bool, Option<String>) {
    match result {
        Ok(out) if out.exit_code == 0 => (true, None),
        Ok(out) => (false, Some(format!("exit code {}", out.exit_code))),
        Err(e) => (false, Some(e.to_string())),
    }
}

/// A new isolate-backed session, the same backend the server uses.
pub async fn new_session() -> Result<sandbox_session::IsolateBackend, sandbox_types::SandboxError> {
    sandbox_session::IsolateBackend::new(create_session_config()).await
}

/// Writes `bytes` to `path` in a session's filesystem, replacing the file.
pub async fn write_file(
    fs: &dyn sandbox_session::session::SessionFs,
    path: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    use sandbox_vfs::OpenFlags;
    let handle = fs
        .open(
            std::path::Path::new(path),
            OpenFlags {
                read: false,
                write: true,
                append: false,
                create: true,
                truncate: true,
                exclusive: false,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;
    let result = fs
        .write(&handle, 0, bytes)
        .await
        .map_err(|e| anyhow::anyhow!("write {path}: {e}"));
    fs.close(&handle);
    result.map(|_| ())
}

/// Reads the whole file at `path` from a session's filesystem.
pub async fn read_file(
    fs: &dyn sandbox_session::session::SessionFs,
    path: &str,
) -> anyhow::Result<Vec<u8>> {
    use sandbox_vfs::OpenFlags;
    let stat = fs
        .stat(std::path::Path::new(path))
        .await
        .map_err(|e| anyhow::anyhow!("stat {path}: {e}"))?;
    let handle = fs
        .open(std::path::Path::new(path), OpenFlags::default())
        .await
        .map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;
    let result = fs
        .read(&handle, 0, stat.size as usize)
        .await
        .map_err(|e| anyhow::anyhow!("read {path}: {e}"));
    fs.close(&handle);
    Ok(result?.to_vec())
}
