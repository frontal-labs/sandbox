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
    write_path(session.fs(), path, data).await
}

/// Creates or replaces the file at `path` in `fs`. The owner of a session's
/// filesystem uses this before the session is running (see `agent_env`).
pub async fn write_path(fs: &dyn SessionFs, path: &str, data: &[u8]) -> Result<(), SandboxError> {
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
    while offset < data.len() {
        let n = fs.write(handle, offset as u64, &data[offset..]).await?;
        if n == 0 {
            break;
        }
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
