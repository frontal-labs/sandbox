//! Tests for the trace file format.

use sandbox_trace::{FileChange, FsOp, Outcome, RunRecord, Trace, TraceError};
use sandbox_types::event::FileOp;
use sandbox_types::{ContentHash, ErrorCode};

fn sample() -> Trace {
    Trace {
        runs: vec![
            RunRecord {
                random_state: None,
                seq: 1,
                script: "echo hi".to_string(),
                outcome: Outcome::Completed {
                    exit_code: 0,
                    stdout: b"hi\n".to_vec(),
                    stderr: Vec::new(),
                },
                files: Vec::new(),
                fs: None,
            },
            RunRecord {
                random_state: None,
                seq: 2,
                script: "echo 'open".to_string(),
                outcome: Outcome::Failed {
                    code: ErrorCode::ShellError,
                    message: "unterminated quote".to_string(),
                },
                files: Vec::new(),
                fs: None,
            },
        ],
    }
}

#[test]
fn a_trace_round_trips_through_its_text_form() {
    let trace = sample();
    assert_eq!(Trace::parse(&trace.to_jsonl().unwrap()).unwrap(), trace);
}

#[test]
fn output_bytes_survive_exactly_including_non_utf8() {
    let trace = Trace {
        runs: vec![RunRecord {
            random_state: None,
            seq: 1,
            script: "x".to_string(),
            outcome: Outcome::Completed {
                exit_code: 3,
                stdout: vec![0, 255, 16, b'\n'],
                stderr: vec![0x80],
            },
            files: Vec::new(),
            fs: None,
        }],
    };
    assert_eq!(Trace::parse(&trace.to_jsonl().unwrap()).unwrap(), trace);
}

#[test]
fn the_header_is_required_and_checked() {
    assert_eq!(Trace::parse(""), Err(TraceError::Empty));
    assert_eq!(
        Trace::parse(r#"{"format":"other","version":1}"#),
        Err(TraceError::BadHeader)
    );
    assert_eq!(
        Trace::parse(r#"{"format":"sandbox-trace","version":9}"#),
        Err(TraceError::UnsupportedVersion(9))
    );
}

#[test]
fn file_changes_round_trip_with_their_hashes() {
    let trace = Trace {
        runs: vec![RunRecord {
            random_state: None,
            seq: 1,
            script: "echo hi > /a.txt".to_string(),
            outcome: Outcome::Completed {
                exit_code: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            files: vec![
                FileChange {
                    path: "/a.txt".to_string(),
                    op: FileOp::Create,
                    before: None,
                    after: Some(ContentHash::new(b"hi\n")),
                },
                FileChange {
                    path: "/d".to_string(),
                    op: FileOp::Mkdir,
                    before: None,
                    after: None,
                },
            ],
            fs: None,
        }],
    };
    let text = trace.to_jsonl().unwrap();
    assert!(text.contains(r#""files""#));
    assert_eq!(Trace::parse(&text).unwrap(), trace);
}

#[test]
fn a_run_without_a_file_list_parses_with_no_files() {
    let text = format!(
        "{}\n{{\"seq\":1,\"script\":\"echo hi\",\"exit_code\":0,\"stdout\":\"6869\",\"stderr\":\"\"}}\n",
        r#"{"format":"sandbox-trace","version":1}"#
    );
    let trace = Trace::parse(&text).unwrap();
    assert!(trace.runs[0].files.is_empty());
}

#[test]
fn filesystem_runs_round_trip_with_their_data() {
    let trace = Trace {
        runs: vec![
            RunRecord {
                random_state: None,
                seq: 1,
                script: String::new(),
                fs: Some(FsOp::Write {
                    path: "/a.txt".to_string(),
                    data: vec![0, 255, b'\n'],
                }),
                outcome: Outcome::Completed {
                    exit_code: 0,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
                files: Vec::new(),
            },
            RunRecord {
                random_state: None,
                seq: 2,
                script: String::new(),
                fs: Some(FsOp::Delete {
                    path: "/a.txt".to_string(),
                }),
                outcome: Outcome::Failed {
                    code: ErrorCode::NotFound,
                    message: "no such file".to_string(),
                },
                files: Vec::new(),
            },
        ],
    };
    let text = trace.to_jsonl().unwrap();
    assert!(text.contains(r#""fs":{"op":"write""#));
    assert_eq!(Trace::parse(&text).unwrap(), trace);
}

#[test]
fn a_filesystem_run_with_a_script_is_refused() {
    let text = format!(
        "{}\n{{\"seq\":1,\"script\":\"echo\",\"exit_code\":0,\"stdout\":\"\",\"stderr\":\"\",\"fs\":{{\"op\":\"delete\",\"path\":\"/a\"}}}}\n",
        r#"{"format":"sandbox-trace","version":1}"#
    );
    assert!(matches!(
        Trace::parse(&text),
        Err(TraceError::BadLine { line: 2, .. })
    ));
}

#[test]
fn a_run_with_no_changes_writes_no_file_list() {
    let trace = sample();
    assert!(!trace.to_jsonl().unwrap().contains(r#""files""#));
}

#[test]
fn a_damaged_line_names_its_line_number() {
    let text = format!(
        "{}\n{{\"seq\":1,\"script\":\"x\",\"exit_code\":0,\"stdout\":\"zz\",\"stderr\":\"\"}}\n",
        r#"{"format":"sandbox-trace","version":1}"#
    );
    match Trace::parse(&text) {
        Err(TraceError::BadLine { line: 2, .. }) => {}
        other => panic!("expected a bad line 2, got {other:?}"),
    }
}

#[test]
fn a_run_with_both_output_and_error_is_refused() {
    let text = format!(
        "{}\n{{\"seq\":1,\"script\":\"x\",\"exit_code\":0,\"stdout\":\"\",\"stderr\":\"\",\"error\":{{\"code\":\"INTERNAL\",\"message\":\"m\"}}}}\n",
        r#"{"format":"sandbox-trace","version":1}"#
    );
    assert!(matches!(
        Trace::parse(&text),
        Err(TraceError::BadLine { line: 2, .. })
    ));
}

#[test]
fn the_random_state_round_trips_and_is_absent_when_unset() {
    let trace = Trace {
        runs: vec![
            RunRecord {
                seq: 1,
                script: "echo $RANDOM".to_string(),
                fs: None,
                outcome: Outcome::Completed {
                    exit_code: 0,
                    stdout: b"7\n".to_vec(),
                    stderr: Vec::new(),
                },
                files: Vec::new(),
                random_state: Some(u64::MAX - 3),
            },
            RunRecord {
                seq: 2,
                script: "true".to_string(),
                fs: None,
                outcome: Outcome::Completed {
                    exit_code: 0,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
                files: Vec::new(),
                random_state: None,
            },
        ],
    };
    let text = trace.to_jsonl().unwrap();
    assert!(text.contains("\"random\":18446744073709551612"), "{text}");
    let parsed = Trace::parse(&text).unwrap();
    assert_eq!(parsed, trace);
    assert_eq!(
        text.matches("\"random\"").count(),
        1,
        "unset state is not written"
    );
}
