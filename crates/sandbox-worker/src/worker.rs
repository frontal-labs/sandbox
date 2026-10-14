//! Worker processes: one child process per session.
//!
//! The host spawns a child that runs `serve` over stdin and stdout. A crash
//! or a hang in the child affects only that worker: the host gets an
//! `Unavailable` error and never panics or loses other sessions. This is
//! process isolation only. Seccomp, cgroups, and namespaces (PLAN.md section
//! 4.13) are Linux-only and not implemented.
//!
//! Protocol: one JSON object per line in each direction. A request is
//! answered by exactly one response line, except `shutdown`, which gets none.

use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{ErrorCode, ExecResult, SandboxError, SessionConfig};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    RunScript { script: String },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { result: ExecResult },
    Error { code: ErrorCode, message: String },
}

/// Child side: answers requests from `input` on `output` until `shutdown` or
/// end of input. Returns an I/O error only if the pipes fail.
pub fn serve(
    session: &dyn Session,
    input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                let response = Response::Error {
                    code: ErrorCode::InvalidArgument,
                    message: format!("malformed request: {e}"),
                };
                write_line(&mut output, &response)?;
                continue;
            }
        };
        let response = match request {
            Request::Shutdown => break,
            Request::RunScript { script } => match rt.block_on(session.run_script(&script)) {
                Ok(result) => Response::Ok { result },
                Err(e) => Response::Error {
                    code: e.code,
                    message: e.message,
                },
            },
        };
        write_line(&mut output, &response)?;
    }
    Ok(())
}

fn write_line(output: &mut impl Write, response: &Response) -> std::io::Result<()> {
    let encoded = serde_json::to_string(response).map_err(std::io::Error::other)?;
    output.write_all(encoded.as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()
}

/// Child entry point for the `sandbox worker` subcommand: a fresh isolate
/// session served over stdin and stdout.
pub fn run_stdio_worker() -> Result<(), SandboxError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SandboxError::internal(format!("failed to start runtime: {e}")))?;
    let session = rt.block_on(IsolateBackend::new(SessionConfig::default()))?;
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    serve(&session, stdin, stdout)
        .map_err(|e| SandboxError::internal(format!("worker I/O failed: {e}")))
}

/// Host side: a handle to one worker process.
pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    /// Starts `program` with `args`. Its stdin and stdout carry the protocol.
    pub fn spawn(
        program: impl AsRef<std::ffi::OsStr>,
        args: &[&str],
    ) -> Result<Self, SandboxError> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| SandboxError::unavailable(format!("failed to start worker: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SandboxError::internal("worker stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SandboxError::internal("worker stdout missing"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// Runs a script in the worker. A dead or unreachable worker returns
    /// `Unavailable`.
    pub fn run_script(&mut self, script: &str) -> Result<ExecResult, SandboxError> {
        let request = serde_json::to_string(&Request::RunScript {
            script: script.to_string(),
        })
        .map_err(|e| SandboxError::internal(format!("failed to encode request: {e}")))?;

        self.stdin
            .write_all(request.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|e| SandboxError::unavailable(format!("worker is not reachable: {e}")))?;

        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| SandboxError::unavailable(format!("worker read failed: {e}")))?;
        if read == 0 {
            return Err(SandboxError::unavailable("worker process exited"));
        }
        match serde_json::from_str::<Response>(&line)
            .map_err(|e| SandboxError::internal(format!("malformed worker response: {e}")))?
        {
            Response::Ok { result } => Ok(result),
            Response::Error { code, message } => Err(SandboxError::new(code, message)),
        }
    }

    /// Whether the child is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Asks the worker to exit, then waits for it.
    pub fn shutdown(mut self) {
        let _ = writeln!(
            self.stdin,
            "{}",
            serde_json::to_string(&Request::Shutdown).unwrap_or_default()
        );
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Reap the child so it does not linger as a zombie.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_session::IsolateBackend;
    use std::io::Cursor;

    fn session() -> IsolateBackend {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(IsolateBackend::new(SessionConfig::default()))
            .unwrap()
    }

    #[test]
    fn serve_answers_each_request_with_one_line() {
        let input = concat!(
            r#"{"op":"run_script","script":"echo hi"}"#,
            "\n",
            r#"{"op":"run_script","script":"false"}"#,
            "\n",
            r#"{"op":"shutdown"}"#,
            "\n",
            r#"{"op":"run_script","script":"echo never"}"#,
            "\n",
        );
        let mut output = Vec::new();
        serve(&session(), Cursor::new(input), &mut output).unwrap();

        let lines: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();
        assert_eq!(lines.len(), 2, "shutdown must stop the loop");
        let first: Response = serde_json::from_str(lines[0]).unwrap();
        match first {
            Response::Ok { result } => assert_eq!(result.stdout, b"hi\n"),
            other => panic!("unexpected {other:?}"),
        }
        let second: Response = serde_json::from_str(lines[1]).unwrap();
        match second {
            Response::Ok { result } => assert_eq!(result.exit_code, 1),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn malformed_requests_get_an_error_response() {
        let mut output = Vec::new();
        serve(&session(), Cursor::new("not json\n"), &mut output).unwrap();
        let response: Response = serde_json::from_slice(&output).unwrap();
        assert!(matches!(
            response,
            Response::Error {
                code: ErrorCode::InvalidArgument,
                ..
            }
        ));
    }

    #[test]
    fn a_dead_worker_is_an_error_not_a_panic() {
        // `true` exits at once, so the first read hits end of file.
        let mut worker = Worker::spawn("true", &[]).unwrap();
        let err = worker.run_script("echo hi").unwrap_err();
        assert_eq!(err.code, ErrorCode::Unavailable);
        assert!(!worker.is_alive());
    }
}
