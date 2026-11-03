use crate::reporter::ScenarioResult;
use crate::scenarios::{
    new_session, read_file, run_with_ramp_up, write_file, ScenarioConfig, WorkerIteration,
    WorkerResult,
};
use anyhow::Result;
use sandbox_session::Session;
use std::time::{Duration, Instant};

const SEED_PATH: &str = "/seed.txt";
const SEED: &[u8] = b"seed";
const FORKS: usize = 3;

pub struct SnapshotForkScenario {
    config: ScenarioConfig,
}

impl SnapshotForkScenario {
    pub fn new(config: ScenarioConfig) -> Self {
        Self { config }
    }

    /// Times one snapshot plus three restores, then checks that each fork reads
    /// back the seeded file. Session setup and the seed write are not timed.
    pub async fn run(self) -> Result<ScenarioResult> {
        let mut result = run_with_ramp_up(&self.config, |worker_id| async move {
            let session = new_session().await?;
            write_file(session.fs(), SEED_PATH, SEED).await?;

            let start = Instant::now();
            let root = session.snapshot().await?;
            let mut outcome: Result<(), String> = Ok(());
            let mut restored = 0;
            while restored < FORKS && outcome.is_ok() {
                restored += 1;
                let fork = sandbox_session::restore(root).await?;
                outcome = match read_file(fork.fs(), SEED_PATH).await {
                    Ok(bytes) if bytes == SEED => Ok(()),
                    Ok(bytes) => Err(format!(
                        "fork read {} bytes, expected the seed",
                        bytes.len()
                    )),
                    Err(e) => Err(e.to_string()),
                };
            }
            let duration: Duration = start.elapsed();
            let (success, error) = match outcome {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e)),
            };
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

        result.scenario = "snapshot_fork".to_string();
        Ok(result)
    }
}
