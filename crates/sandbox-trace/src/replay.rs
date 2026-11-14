//! Replays a trace and compares each run with the recorded outcome. See ADR 0007.
//!
//! The caller supplies the function that runs one recorded run, so this crate
//! does not depend on a session implementation.

use crate::format::{FsOp, Outcome, RunRecord, Trace};
use sandbox_types::SandboxError;
use std::future::Future;

/// A run whose outcome differs from the recorded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub seq: u64,
    /// The script. Empty for a filesystem run, which has `fs` instead.
    pub script: String,
    pub fs: Option<FsOp>,
    pub expected: Outcome,
    pub actual: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayReport {
    /// How many scripts were run.
    pub runs: usize,
    pub divergences: Vec<Divergence>,
}

impl ReplayReport {
    /// True when every run produced the recorded outcome.
    pub fn matches(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// The outcome of running one script: its exit code and output, or the error
/// that stopped it before it ran.
pub fn outcome_of(result: Result<(i32, Vec<u8>, Vec<u8>), SandboxError>) -> Outcome {
    match result {
        Ok((exit_code, stdout, stderr)) => Outcome::Completed {
            exit_code,
            stdout,
            stderr,
        },
        Err(error) => Outcome::Failed {
            code: error.code,
            message: error.message,
        },
    }
}

/// Runs every run of `trace`, in order, through `run`, and compares each outcome
/// with the recorded one. `run` receives the whole record, so it can tell a
/// script from a filesystem write (`RunRecord::fs`). Runs continue after a
/// divergence, so the report covers the whole trace.
pub async fn replay<F, Fut>(trace: &Trace, mut run: F) -> ReplayReport
where
    F: FnMut(&RunRecord) -> Fut,
    Fut: Future<Output = Result<(i32, Vec<u8>, Vec<u8>), SandboxError>>,
{
    let mut report = ReplayReport {
        runs: trace.runs.len(),
        divergences: Vec::new(),
    };
    for record in &trace.runs {
        let actual = outcome_of(run(record).await);
        if actual != record.outcome {
            report.divergences.push(Divergence {
                seq: record.seq,
                script: record.script.clone(),
                fs: record.fs.clone(),
                expected: record.outcome.clone(),
                actual,
            });
        }
    }
    report
}
