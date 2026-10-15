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

/// Commands the isolate shell runs. The set is deliberately limited to builtins,
/// because host tools such as git or cargo are not available in the isolate.
const WORKLOAD: &[&str] = &[
    "echo hello",
    "pwd",
    "echo one two three",
    "ls /",
    "true",
    "false",
];

pub struct MixedWorkloadScenario {
    config: ScenarioConfig,
}

impl MixedWorkloadScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Each iteration starts a session and runs one command from the workload.
    /// `false` exits 1, so it counts as a failed iteration. That keeps the error
    /// rate honest about what the workload does.
    pub async fn run(self) -> Result<ScenarioResult> {
        // Every iteration takes the next command, so the whole list runs no
        // matter how many workers there are.
        let next = Arc::new(AtomicUsize::new(0));
        let mut result = run_with_ramp_up(&self.config, move |worker_id| {
            let next = next.clone();
            async move {
                let command = WORKLOAD[next.fetch_add(1, Ordering::Relaxed) % WORKLOAD.len()];
                let session = new_session().await?;
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

        result.scenario = "mixed_workload".to_string();
        Ok(result)
    }
}
