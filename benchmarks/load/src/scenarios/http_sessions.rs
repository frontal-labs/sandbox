use crate::reporter::{LatencyHistogram, ScenarioResult};
use crate::scenarios::{create_session_config, ScenarioConfig};
use anyhow::{bail, Result};
use frontal_sandbox_sdk::Client;
use sandbox_types::{SessionId, TenantId};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Sessions assigned to one tenant. The server caps a tenant at 256, so a run that holds more
/// than that spreads its sessions over several tenants. `HTTP_TENANT_SPLIT=0` puts every
/// session in the default tenant instead, to see the cap.
const SESSIONS_PER_TENANT: usize = 200;
/// Pause between one session's commands, in milliseconds. `HOLD_THINK_MS` overrides it.
const DEFAULT_THINK_MS: u64 = 100;

pub struct HttpSessionsScenario {
    config: ScenarioConfig,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Counts failures by message, so a run that ends in 429s says so.
#[derive(Default)]
struct ErrorTally(BTreeMap<String, usize>);

impl ErrorTally {
    fn add(&mut self, message: String) {
        // Session ids and counts vary; the first 80 characters name the failure.
        let key: String = message.chars().take(80).collect();
        *self.0.entry(key).or_default() += 1;
    }

    fn merge(&mut self, other: ErrorTally) {
        for (k, v) in other.0 {
            *self.0.entry(k).or_default() += v;
        }
    }

    fn total(&self) -> usize {
        self.0.values().sum()
    }
}

impl HttpSessionsScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Like `held_sessions`, over HTTP: `concurrency` sessions are created on the server at
    /// `--server-url`, spread over the ramp-up. All that were admitted stay live, each runs
    /// `echo ready` once per think time for the duration, and then they are deleted.
    /// Keys come from `SANDBOX_API_KEYS` (comma separated) or `SANDBOX_API_KEY`, and
    /// sessions are spread across them.
    pub async fn run(self) -> Result<ScenarioResult> {
        let Some(url) = self.config.server_url.clone() else {
            bail!("http_sessions needs --server-url");
        };
        let keys: Vec<String> = std::env::var("SANDBOX_API_KEYS")
            .or_else(|_| std::env::var("SANDBOX_API_KEY"))
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect();
        if keys.is_empty() {
            bail!("set SANDBOX_API_KEYS (comma separated) or SANDBOX_API_KEY");
        }
        let clients: Vec<Client> = keys
            .iter()
            .map(|k| Client::new(url.clone(), Some(k.clone())))
            .collect();
        let n = self.config.concurrency;
        let think = Duration::from_millis(env_u64("HOLD_THINK_MS", DEFAULT_THINK_MS));
        let split = env_u64("HTTP_TENANT_SPLIT", 1) != 0;
        let tenants: Vec<TenantId> = (0..n.div_ceil(SESSIONS_PER_TENANT).max(1))
            .map(|_| TenantId::new())
            .collect();

        // Phase 1: create.
        let created_at = Instant::now();
        let ramp_step = if n > 0 {
            self.config.ramp_up / n as u32
        } else {
            Duration::ZERO
        };
        let mut creating = Vec::with_capacity(n);
        for i in 0..n {
            let client = clients[i % clients.len()].clone();
            let mut session_config = create_session_config();
            if split {
                session_config.tenant_id = tenants[i / SESSIONS_PER_TENANT];
            }
            creating.push(tokio::spawn(async move {
                let delay = ramp_step * i as u32;
                if delay > Duration::ZERO {
                    sleep(delay).await;
                }
                let start = Instant::now();
                let outcome = client.create_session(session_config).await;
                (client, outcome.map_err(|e| e.to_string()), start.elapsed())
            }));
        }
        let mut live: Vec<(Client, SessionId)> = Vec::new();
        let mut create_ns = Vec::with_capacity(n);
        let mut failed_create_ns = Vec::new();
        let mut errors = ErrorTally::default();
        for handle in creating {
            let (client, outcome, took) = handle.await?;
            match outcome {
                Ok(id) => {
                    live.push((client, id));
                    create_ns.push(took.as_nanos() as u64);
                }
                Err(e) => {
                    failed_create_ns.push(took.as_nanos() as u64);
                    errors.add(format!("create: {e}"));
                }
            }
        }
        let all_created = created_at.elapsed();
        let admitted = live.len();
        println!(
            "http_sessions: {admitted}/{n} sessions admitted after {:.2}s over {} key(s), {} create failures",
            all_created.as_secs_f64(),
            clients.len(),
            n - admitted
        );

        // Phase 2: exec on every live session.
        let exec_start = Instant::now();
        let deadline = exec_start + self.config.duration;
        let mut driving = Vec::with_capacity(admitted);
        for (i, (client, id)) in live.iter().cloned().enumerate() {
            driving.push(tokio::spawn(async move {
                sleep(think.mul_f64(i as f64 / admitted.max(1) as f64)).await;
                let mut samples = Vec::new();
                let mut tally = ErrorTally::default();
                while Instant::now() < deadline {
                    let start = Instant::now();
                    match client.exec(id, "echo ready").await {
                        Ok(out) if out.exit_code == 0 => {}
                        Ok(out) => tally.add(format!("exec: exit code {}", out.exit_code)),
                        Err(e) => tally.add(format!("exec: {e}")),
                    }
                    samples.push(start.elapsed().as_nanos() as u64);
                    sleep(think).await;
                }
                (samples, tally)
            }));
        }
        let mut exec_ns = Vec::new();
        for handle in driving {
            let (samples, tally) = handle.await?;
            exec_ns.extend(samples);
            errors.merge(tally);
        }
        let exec_elapsed = exec_start.elapsed();

        // Phase 3: delete.
        let destroy_start = Instant::now();
        let mut destroying = Vec::with_capacity(admitted);
        for (client, id) in live {
            destroying.push(tokio::spawn(async move {
                let start = Instant::now();
                let outcome = client.destroy_session(id).await;
                (outcome.map_err(|e| e.to_string()), start.elapsed())
            }));
        }
        let mut destroy_ns = Vec::with_capacity(admitted);
        for handle in destroying {
            let (outcome, took) = handle.await?;
            match outcome {
                Ok(()) => destroy_ns.push(took.as_nanos() as u64),
                Err(e) => errors.add(format!("destroy: {e}")),
            }
        }
        println!(
            "http_sessions: {} sessions deleted in {:.2}s",
            destroy_ns.len(),
            destroy_start.elapsed().as_secs_f64()
        );
        if errors.total() > 0 {
            println!("http_sessions: errors by message:");
            for (message, count) in &errors.0 {
                println!("  {count:>8}  {message}");
            }
        }

        let total_errors = errors.total();
        let total_iterations = n + exec_ns.len() + destroy_ns.len();
        let mut latencies = HashMap::new();
        latencies.insert("create".to_string(), LatencyHistogram::from_samples(&create_ns));
        latencies.insert(
            "create_failed".to_string(),
            LatencyHistogram::from_samples(&failed_create_ns),
        );
        latencies.insert("exec".to_string(), LatencyHistogram::from_samples(&exec_ns));
        latencies.insert(
            "destroy".to_string(),
            LatencyHistogram::from_samples(&destroy_ns),
        );
        latencies.insert("overall".to_string(), LatencyHistogram::from_samples(&exec_ns));
        let mut throughput = HashMap::new();
        throughput.insert(
            "overall".to_string(),
            exec_ns.len() as f64 / exec_elapsed.as_secs_f64(),
        );
        throughput.insert(
            "create".to_string(),
            admitted as f64 / all_created.as_secs_f64().max(f64::EPSILON),
        );
        let mut error_rates = HashMap::new();
        error_rates.insert(
            "overall".to_string(),
            total_errors as f64 / total_iterations.max(1) as f64,
        );

        Ok(ScenarioResult {
            scenario: "http_sessions".to_string(),
            duration_secs: all_created.as_secs_f64() + exec_elapsed.as_secs_f64(),
            concurrency: admitted,
            total_iterations,
            total_errors,
            latencies,
            throughput,
            error_rates,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }
}
