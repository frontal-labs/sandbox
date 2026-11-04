//! Promotion, Tier 1 to Tier 2 (PLAN.md section 4.11): the session's files are copied into
//! a running guest under the same paths, and the guest runs the commands from then on.
//! Processes do not survive promotion, so only the files move. The copy is one-way for now.
//!
//! Symlinks are refused rather than skipped. A guest that quietly dropped a link would run
//! commands against a different tree from the one the session showed.

use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::guest::{MkDirRequest, WriteFileRequest};
use sandbox_session::Session;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{OpenFlags, SessionFs};
use std::path::Path;
use tonic::transport::Channel;

/// Bytes moved per message. Well under the transport's limit, and above the agent's
/// default chunk, so a large file takes few round trips.
pub const CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PromotionReport {
    pub directories: u64,
    pub files: u64,
    pub bytes: u64,
}

/// Copies every directory and file under `/` into the guest, parents before children.
pub async fn promote_tree(
    fs: &dyn SessionFs,
    guest: &mut GuestServiceClient<Channel>,
) -> Result<PromotionReport, SandboxError> {
    let mut report = PromotionReport::default();
    let mut pending = vec!["/".to_string()];
    while let Some(dir) = pending.pop() {
        let mut entries = fs
            .readdir(Path::new(&dir))
            .await
            .map_err(|e| vfs_error(&e))?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            let path = join(&dir, &entry.name);
            if entry.is_symlink {
                return Err(SandboxError::new(
                    ErrorCode::Unsupported,
                    format!(
                        "cannot promote the symlink {path}: links are not carried into a guest yet"
                    ),
                ));
            }
            if entry.is_dir {
                guest
                    .mk_dir(MkDirRequest { path: path.clone() })
                    .await
                    .map_err(guest_error)?;
                report.directories += 1;
                pending.push(path);
            } else if entry.is_file {
                copy_file(fs, guest, &path, &mut report).await?;
            }
        }
    }
    Ok(report)
}

/// Copies one file in chunks. An empty file still gets one write, so it exists in the guest.
async fn copy_file(
    fs: &dyn SessionFs,
    guest: &mut GuestServiceClient<Channel>,
    path: &str,
    report: &mut PromotionReport,
) -> Result<(), SandboxError> {
    let handle = fs
        .open(Path::new(path), OpenFlags::default())
        .await
        .map_err(|e| vfs_error(&e))?;
    let mut offset: u64 = 0;
    let mut finished = false;
    let result: Result<(), SandboxError> = async {
        while !finished {
            let chunk = fs
                .read(&handle, offset, CHUNK_BYTES)
                .await
                .map_err(|e| vfs_error(&e))?;
            finished = chunk.len() < CHUNK_BYTES;
            if !chunk.is_empty() || offset == 0 {
                guest
                    .write_file(WriteFileRequest {
                        path: path.to_string(),
                        data: chunk.to_vec(),
                        offset,
                    })
                    .await
                    .map_err(guest_error)?;
            }
            offset += chunk.len() as u64;
        }
        Ok(())
    }
    .await;
    fs.close(&handle);
    result?;
    report.files += 1;
    report.bytes += offset;
    Ok(())
}

/// Promotes a session's files into the guest at `agent`, such as `http://127.0.0.1:7000`.
pub async fn promote_session(
    session: &dyn Session,
    agent: &str,
) -> Result<PromotionReport, SandboxError> {
    let mut guest = GuestServiceClient::connect(agent.to_string())
        .await
        .map_err(|e| {
            SandboxError::new(
                ErrorCode::VmBootFailed,
                format!("cannot reach the guest agent: {e}"),
            )
        })?;
    promote_tree(session.fs(), &mut guest).await
}

fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn vfs_error(error: &sandbox_vfs::VfsError) -> SandboxError {
    SandboxError::new(ErrorCode::VfsError, error.to_string())
}

fn guest_error(status: tonic::Status) -> SandboxError {
    SandboxError::new(
        ErrorCode::VmBootFailed,
        format!("the guest refused a write: {}", status.message()),
    )
}
