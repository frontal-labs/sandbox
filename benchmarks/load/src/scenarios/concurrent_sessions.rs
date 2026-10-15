use crate::reporter::ScenarioResult;
use crate::scenarios::{
    create_exec_request, exec_outcome, new_session, run_with_ramp_up, ScenarioConfig,
    WorkerIteration, WorkerResult,
};
use anyhow::Result;
use sandbox_session::Session;
use std::time::Instant;

pub struct ConcurrentSessionsScenario {
    config: ScenarioConfig,
}

impl ConcurrentSessionsScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Each iteration creates a session, runs one command, and destroys it. The
    /// workers do this at the same time, so the session lifecycle is measured
    /// under concurrency.
    pub async fn run(self) -> Result<ScenarioResult> {
        let config = self.config;
        let mut result = run_with_ramp_up(&config, move |worker_id| async move {
            let start = Instant::now();
            let (success, error) = match new_session().await {
                Ok(session) => {
                    let outcome =
                        exec_outcome(session.exec(create_exec_request("echo ready")).await);
                    let _ = Box::new(session).destroy().await;
                    outcome
                }
                Err(e) => (false, Some(e.to_string())),
            };
            let duration = start.elapsed();
            Ok(WorkerResult {
                worker_id,
                iterations: vec![WorkerIteration {
                    duration,
                    success,
                    error,
                }],
                total_duration: duration,
            })
        })
        .await?;

        result.scenario = "concurrent_sessions".to_string();
        Ok(result)
    }
}
