//! Tests for trace replay.

use sandbox_trace::{replay, FsOp, Outcome, RunRecord, Trace};
use sandbox_types::{ErrorCode, SandboxError};

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
