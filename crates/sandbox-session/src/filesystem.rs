//! Filesystem writes made through the API, and replay of recorded runs (ADR 0012).
//!
//! The server and trace replay both call these functions, so a replayed write
//! takes the same path as the original.

use crate::session::{Session, SessionFs};
use sandbox_trace::{FsOp, RunRecord};
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{OpenFlags, VfsError};
use std::path::Path;

/// Maps a VFS error to the API error code it is reported with.
pub fn vfs_error(error: VfsError) -> SandboxError {
    match error {
        VfsError::NotFound(m) => SandboxError::not_found(m),
        VfsError::AlreadyExists(m) => SandboxError::already_exists(m),
        VfsError::InvalidPath(m) => SandboxError::invalid_argument(m),
        VfsError::PermissionDenied(m) => SandboxError::permission_denied(m),
        VfsError::QuotaExceeded(m) => SandboxError::resource_exhausted(m),
        VfsError::Unsupported(m) => SandboxError::unsupported(m),
        other => SandboxError::new(ErrorCode::VfsError, other.to_string()),
    }
}

/// Creates or replaces the file at `path` with `data`.
pub async fn write_file(
    session: &dyn Session,
    path: &str,
    data: &[u8],
) -> Result<(), SandboxError> {
    let fs = session.fs();
    let flags = OpenFlags {
        read: false,
        write: true,
        append: false,
        create: true,
        truncate: true,
        exclusive: false,
    };
    let handle = fs.open(Path::new(path), flags).await.map_err(vfs_error)?;
    let written = write_all(fs, &handle, data).await;
    fs.close(&handle);
    written.map_err(vfs_error)
}

/// Removes the file or empty directory at `path`.
pub async fn delete_file(session: &dyn Session, path: &str) -> Result<(), SandboxError> {
    session
        .fs()
        .rm(Path::new(path), false)
        .await
        .map_err(vfs_error)
}

/// Writes every byte, since one VFS write may accept only part of the buffer.
async fn write_all(
    fs: &dyn SessionFs,
    handle: &sandbox_vfs::FileHandle,
    data: &[u8],
) -> Result<(), VfsError> {
    let mut offset = 0usize;
    // A write that takes no bytes ends the loop, so a stalled file cannot spin it.
    let mut progressed = true;
    while progressed && offset < data.len() {
        let n = fs.write(handle, offset as u64, &data[offset..]).await?;
        progressed = n > 0;
        offset += n;
    }
    Ok(())
}

/// Runs one recorded run in `session` and returns its exit code and output, the
/// same shape a script run produces. A filesystem run returns exit code 0 and no
/// output.
pub async fn replay_run(
    session: &dyn Session,
    run: &RunRecord,
) -> Result<(i32, Vec<u8>, Vec<u8>), SandboxError> {
    // A run that recorded its $RANDOM state draws the same values again.
    if let Some(state) = run.random_state {
        session.set_random_state(state).await;
    }
    match &run.fs {
        Some(FsOp::Write { path, data }) => write_file(session, path, data)
            .await
            .map(|()| (0, Vec::new(), Vec::new())),
        Some(FsOp::Delete { path }) => delete_file(session, path)
            .await
            .map(|()| (0, Vec::new(), Vec::new())),
        None => session
            .run_script(&run.script)
            .await
            .map(|out| (out.exit_code, out.stdout, out.stderr)),
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::backend::IsolateBackend;
    use sandbox_trace::{Outcome, RunRecord};
    use sandbox_types::SessionConfig;

    #[tokio::test]
    async fn a_replayed_run_draws_the_same_random_values() {
        let original = IsolateBackend::new(SessionConfig::default()).await.unwrap();
        let state = original.random_state().await;
        let out = original.run_script("echo $RANDOM $RANDOM").await.unwrap();
        let run = RunRecord {
            seq: 1,
            script: "echo $RANDOM $RANDOM".to_string(),
            fs: None,
            outcome: Outcome::Completed {
                exit_code: out.exit_code,
                stdout: out.stdout.clone(),
                stderr: out.stderr.clone(),
            },
            files: Vec::new(),
            random_state: Some(state),
        };

        // A fresh session starts from a different clock seed, so it would draw other
        // values without the recorded state.
        let fresh = IsolateBackend::new(SessionConfig::default()).await.unwrap();
        let (code, stdout, _) = replay_run(&fresh, &run).await.unwrap();
        assert_eq!(code, out.exit_code);
        assert_eq!(
            stdout, out.stdout,
            "the replay reproduces the recorded $RANDOM values"
        );
    }
}
