//! The trace file format. See ADR 0007.
//!
//! A trace is JSON Lines. The first line is the header. Every later line is one
//! run, in the order it ran. A completed run records its exit code and its exact
//! output, and a run that failed before it ran records the error.

use sandbox_types::ErrorCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const FORMAT: &str = "sandbox-trace";
pub const VERSION: u32 = 1;

/// What one script did when it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed {
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Failed {
        code: ErrorCode,
        message: String,
    },
}

/// One script in the order it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub seq: u64,
    pub script: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub runs: Vec<RunRecord>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TraceError {
    #[error("the trace is empty; it needs a header line")]
    Empty,
    #[error("line 1 is not a sandbox-trace header")]
    BadHeader,
    #[error("unsupported trace version {0}; this build reads version {VERSION}")]
    UnsupportedVersion(u32),
    #[error("line {line}: {message}")]
    BadLine { line: usize, message: String },
}

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    version: u32,
}

#[derive(Serialize, Deserialize)]
struct Line {
    seq: u64,
    script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<LineError>,
}

#[derive(Serialize, Deserialize)]
struct LineError {
    code: ErrorCode,
    message: String,
}

impl Trace {
    /// Renders the trace, header first, with a newline after every line.
    pub fn to_jsonl(&self) -> String {
        let header = Header {
            format: FORMAT.to_string(),
            version: VERSION,
        };
        let mut out = String::new();
        out.push_str(&serde_json::to_string(&header).expect("header serializes"));
        out.push('\n');
        for run in &self.runs {
            let line = match &run.outcome {
                Outcome::Completed {
                    exit_code,
                    stdout,
                    stderr,
                } => Line {
                    seq: run.seq,
                    script: run.script.clone(),
                    exit_code: Some(*exit_code),
                    stdout: Some(hex_encode(stdout)),
                    stderr: Some(hex_encode(stderr)),
                    error: None,
                },
                Outcome::Failed { code, message } => Line {
                    seq: run.seq,
                    script: run.script.clone(),
                    exit_code: None,
                    stdout: None,
                    stderr: None,
                    error: Some(LineError {
                        code: *code,
                        message: message.clone(),
                    }),
                },
            };
            out.push_str(&serde_json::to_string(&line).expect("run line serializes"));
            out.push('\n');
        }
        out
    }

    /// Parses a trace. Blank lines after the header are ignored.
    pub fn parse(text: &str) -> Result<Self, TraceError> {
        let mut lines = text.lines().enumerate();
        let (_, first) = lines.next().ok_or(TraceError::Empty)?;
        let header: Header = serde_json::from_str(first).map_err(|_| TraceError::BadHeader)?;
        if header.format != FORMAT {
            return Err(TraceError::BadHeader);
        }
        if header.version != VERSION {
            return Err(TraceError::UnsupportedVersion(header.version));
        }

        let mut runs = Vec::new();
        for (index, text) in lines {
            let line_no = index + 1;
            if text.trim().is_empty() {
                continue;
            }
            let bad = |message: String| TraceError::BadLine {
                line: line_no,
                message,
            };
            let parsed: Line = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
            let outcome = match (parsed.exit_code, parsed.stdout, parsed.stderr, parsed.error) {
                (Some(exit_code), Some(stdout), Some(stderr), None) => Outcome::Completed {
                    exit_code,
                    stdout: hex_decode(&stdout).map_err(bad)?,
                    stderr: hex_decode(&stderr).map_err(bad)?,
                },
                (None, None, None, Some(error)) => Outcome::Failed {
                    code: error.code,
                    message: error.message,
                },
                _ => {
                    return Err(bad(
                        "a run must have either exit_code, stdout, and stderr, or error"
                            .to_string(),
                    ))
                }
            };
            runs.push(RunRecord {
                seq: parsed.seq,
                script: parsed.script,
                outcome,
            });
        }
        Ok(Self { runs })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex output has an odd number of digits".to_string());
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(format!("'{}' is not a hex digit", c as char)),
            };
            Ok(digit(pair[0])? << 4 | digit(pair[1])?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        Trace {
            runs: vec![
                RunRecord {
                    seq: 1,
                    script: "echo hi".to_string(),
                    outcome: Outcome::Completed {
                        exit_code: 0,
                        stdout: b"hi\n".to_vec(),
                        stderr: Vec::new(),
                    },
                },
                RunRecord {
                    seq: 2,
                    script: "echo 'open".to_string(),
                    outcome: Outcome::Failed {
                        code: ErrorCode::ShellError,
                        message: "unterminated quote".to_string(),
                    },
                },
            ],
        }
    }

    #[test]
    fn a_trace_round_trips_through_its_text_form() {
        let trace = sample();
        assert_eq!(Trace::parse(&trace.to_jsonl()).unwrap(), trace);
    }

    #[test]
    fn output_bytes_survive_exactly_including_non_utf8() {
        let trace = Trace {
            runs: vec![RunRecord {
                seq: 1,
                script: "x".to_string(),
                outcome: Outcome::Completed {
                    exit_code: 3,
                    stdout: vec![0, 255, 16, b'\n'],
                    stderr: vec![0x80],
                },
            }],
        };
        assert_eq!(Trace::parse(&trace.to_jsonl()).unwrap(), trace);
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
}
