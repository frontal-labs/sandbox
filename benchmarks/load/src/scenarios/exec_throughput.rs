use crate::reporter::ScenarioResult;
use crate::scenarios::{
    create_exec_request, exec_outcome, new_session, run_with_ramp_up, ScenarioConfig,
    WorkerIteration, WorkerResult,
};
use anyhow::Result;
use sandbox_session::Session;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Commands cycled across iterations. Each one runs in the isolate shell and exits 0.
const COMMANDS: &[&str] = &["echo hello", "pwd", "echo one two three", "ls /"];

pub struct ExecThroughputScenario {
    config: ScenarioConfig,
}

impl ExecThroughputScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Times only `exec`. Session creation is outside the timed region.
    pub async fn run(self) -> Result<ScenarioResult> {
        let next = Arc::new(AtomicUsize::new(0));
        let mut result = run_with_ramp_up(&self.config, move |worker_id| {
            let next = next.clone();
            async move {
                let session = match new_session().await {
                    Ok(session) => session,
                    Err(e) => {
                        return Ok(failed(worker_id, e.to_string()));
                    }
                };
                let command = COMMANDS[next.fetch_add(1, Ordering::Relaxed) % COMMANDS.len()];
                let start = Instant::now();
                let outcome = exec_outcome(session.exec(create_exec_request(command)).await);
                let duration = start.elapsed();
                let _ = Box::new(session).destroy().await;
                Ok(WorkerResult {
                    worker_id,
                    iterations: vec![WorkerIteration {
                        duration,
                        success: outcome.0,
                        error: outcome.1,
                    }],
                    total_duration: duration,
                })
            }
        })
        .await?;

        result.scenario = "exec_throughput".to_string();
        Ok(result)
    }
}

fn failed(worker_id: usize, error: String) -> WorkerResult {
    WorkerResult {
        worker_id,
        iterations: vec![WorkerIteration {
            duration: std::time::Duration::ZERO,
            success: false,
            error: Some(error),
        }],
        total_duration: std::time::Duration::ZERO,
    }
}
