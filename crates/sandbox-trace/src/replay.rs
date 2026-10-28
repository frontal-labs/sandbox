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

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_types::ErrorCode;

    fn trace() -> Trace {
        Trace {
            runs: vec![
                RunRecord {
                    random_state: None,
                    seq: 1,
                    script: "echo a".to_string(),
                    outcome: Outcome::Completed {
                        exit_code: 0,
                        stdout: b"a\n".to_vec(),
                        stderr: Vec::new(),
                    },
                    files: Vec::new(),
                    fs: None,
                },
                RunRecord {
                    random_state: None,
                    seq: 2,
                    script: "echo b".to_string(),
                    outcome: Outcome::Completed {
                        exit_code: 0,
                        stdout: b"b\n".to_vec(),
                        stderr: Vec::new(),
                    },
                    files: Vec::new(),
                    fs: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn a_faithful_run_matches() {
        let report = replay(&trace(), |run: &RunRecord| {
            let script = run.script.clone();
            async move {
                let mut out = script.trim_start_matches("echo ").as_bytes().to_vec();
                out.push(b'\n');
                Ok((0, out, Vec::new()))
            }
        })
        .await;
        assert!(report.matches(), "{:?}", report.divergences);
        assert_eq!(report.runs, 2);
    }

    #[tokio::test]
    async fn a_different_output_is_reported_with_both_sides() {
        let report = replay(&trace(), |run: &RunRecord| {
            let script = run.script.clone();
            async move {
                let out: &[u8] = if script == "echo a" {
                    b"a\n"
                } else {
                    b"changed\n"
                };
                Ok((0, out.to_vec(), Vec::new()))
            }
        })
        .await;
        assert_eq!(report.divergences.len(), 1);
        let d = &report.divergences[0];
        assert_eq!(d.seq, 2);
        assert_eq!(d.script, "echo b");
        assert_eq!(
            d.actual,
            Outcome::Completed {
                exit_code: 0,
                stdout: b"changed\n".to_vec(),
                stderr: Vec::new()
            }
        );
    }

    #[tokio::test]
    async fn an_error_where_output_was_recorded_is_a_divergence() {
        let report = replay(&trace(), |_: &RunRecord| async {
            Err(SandboxError::new(ErrorCode::ShellError, "boom"))
        })
        .await;
        assert_eq!(
            report.divergences.len(),
            2,
            "every run is checked, not just the first"
        );
        assert!(matches!(
            report.divergences[0].actual,
            Outcome::Failed {
                code: ErrorCode::ShellError,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_filesystem_run_reaches_the_callback_with_its_operation() {
        let mut trace = trace();
        trace.runs[1] = RunRecord {
            random_state: None,
            seq: 2,
            script: String::new(),
            fs: Some(FsOp::Write {
                path: "/a".to_string(),
                data: b"x".to_vec(),
            }),
            outcome: Outcome::Completed {
                exit_code: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            files: Vec::new(),
        };
        let report = replay(&trace, |run: &RunRecord| {
            let fs = run.fs.clone();
            async move {
                match fs {
                    Some(FsOp::Write { path, data }) => {
                        assert_eq!((path.as_str(), data.as_slice()), ("/a", b"x".as_slice()));
                        Ok((0, Vec::new(), Vec::new()))
                    }
                    Some(FsOp::Delete { .. }) => panic!("no delete was recorded"),
                    None => Ok((0, b"a\n".to_vec(), Vec::new())),
                }
            }
        })
        .await;
        assert!(report.matches(), "{:?}", report.divergences);
    }
}
