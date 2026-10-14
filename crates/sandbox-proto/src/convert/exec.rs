use super::invalid;
use crate::v1::types as pb;
use sandbox_types::{ExecRequest, ExecResult, SandboxError};
use std::collections::HashMap;

impl From<ExecRequest> for pb::ExecRequest {
    fn from(req: ExecRequest) -> Self {
        Self {
            argv: req.argv,
            env: req.env.into_iter().collect(),
            cwd: req.cwd,
            stdin: req.stdin,
            timeout_ms: req.timeout_ms,
            stream: req.stream,
        }
    }
}

impl TryFrom<pb::ExecRequest> for ExecRequest {
    type Error = SandboxError;
    fn try_from(req: pb::ExecRequest) -> Result<Self, Self::Error> {
        if req.argv.is_empty() {
            return Err(invalid("argv must not be empty"));
        }
        Ok(Self {
            argv: req.argv,
            env: req.env.into_iter().collect::<HashMap<_, _>>(),
            cwd: req.cwd,
            stdin: req.stdin,
            timeout_ms: req.timeout_ms,
            stream: req.stream,
        })
    }
}

impl From<ExecResult> for pb::ExecResult {
    fn from(res: ExecResult) -> Self {
        Self {
            exit_code: res.exit_code,
            stdout: res.stdout,
            stderr: res.stderr,
            cpu_ms: res.cpu_ms,
            wall_ms: res.wall_ms,
        }
    }
}

impl TryFrom<pb::ExecResult> for ExecResult {
    type Error = SandboxError;
    fn try_from(res: pb::ExecResult) -> Result<Self, Self::Error> {
        Ok(Self {
            exit_code: res.exit_code,
            stdout: res.stdout,
            stderr: res.stderr,
            cpu_ms: res.cpu_ms,
            wall_ms: res.wall_ms,
        })
    }
}
