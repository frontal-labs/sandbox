//! Exports a session's filesystem as an ext4 image (PLAN.md section 4.11, `export_ext4`). The
//! tree is written to a scratch directory, and `mkfs.ext4 -d` populates an image from it, so no
//! mount or privilege is needed. The image is what a VM's root disk holds when a guest boots
//! from the session's files rather than from a copy over the protocol.
//!
//! Symbolic links are refused, as promotion refuses them: an image that dropped a link would
//! not match the session. Device nodes and sockets do not exist in the VFS.

use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{OpenFlags, SessionFs};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReport {
    pub path: PathBuf,
    pub size_mib: u64,
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
}

/// Writes `fs` to `image` as an ext4 filesystem, labelled `root`. The image is sized from the
/// tree's contents plus room for the filesystem's own metadata, and at least 16 MiB.
pub async fn export_ext4(fs: &dyn SessionFs, image: &Path, scratch: &Path) -> Result<ImageReport, SandboxError> {
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).map_err(io_error)?;
    let mut totals = Totals::default();
    let outcome = copy_tree(fs, scratch, &mut totals).await;
    let result = match outcome {
        Ok(()) => {
            let size_mib = (totals.bytes / (1024 * 1024) * 3 / 2 + 8).max(16);
            run_mkfs(scratch, image, size_mib)?;
            Ok(ImageReport {
                path: image.to_path_buf(),
                size_mib,
                files: totals.files,
                directories: totals.directories,
                bytes: totals.bytes,
            })
        }
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_dir_all(scratch);
    result
}

#[derive(Default)]
struct Totals {
    files: u64,
    directories: u64,
    bytes: u64,
}

/// Copies every directory and file under `/` into `root`, parents before children.
async fn copy_tree(fs: &dyn SessionFs, root: &Path, totals: &mut Totals) -> Result<(), SandboxError> {
    let mut pending = vec!["/".to_string()];
    while let Some(dir) = pending.pop() {
        let mut entries = fs
            .readdir(Path::new(&dir))
            .await
            .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            let path = if dir == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_symlink {
                return Err(SandboxError::new(
                    ErrorCode::Unsupported,
                    format!("cannot export the symlink {path}: links are not carried into an image yet"),
                ));
            }
            let host = root.join(path.trim_start_matches('/'));
            if entry.is_dir {
                std::fs::create_dir_all(&host).map_err(io_error)?;
                totals.directories += 1;
                pending.push(path);
            } else if entry.is_file {
                let data = read_all(fs, &path).await?;
                totals.bytes += data.len() as u64;
                totals.files += 1;
                std::fs::write(&host, data).map_err(io_error)?;
            }
        }
    }
    Ok(())
}

async fn read_all(fs: &dyn SessionFs, path: &str) -> Result<Vec<u8>, SandboxError> {
    const CHUNK: usize = 1024 * 1024;
    let handle = fs
        .open(Path::new(path), OpenFlags::default())
        .await
        .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
    let mut out = Vec::new();
    let mut finished = false;
    while !finished {
        let chunk = fs
            .read(&handle, out.len() as u64, CHUNK)
            .await
            .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()));
        match chunk {
            Ok(chunk) => {
                finished = chunk.len() < CHUNK;
                out.extend_from_slice(&chunk);
            }
            Err(error) => {
                fs.close(&handle);
                return Err(error);
            }
        }
    }
    fs.close(&handle);
    Ok(out)
}

/// Runs `mkfs.ext4 -d`, which fills the new image from the scratch directory.
fn run_mkfs(root: &Path, image: &Path, size_mib: u64) -> Result<(), SandboxError> {
    let _ = std::fs::remove_file(image);
    let output = Command::new("mkfs.ext4")
        .arg("-q")
        .arg("-F")
        .arg("-L")
        .arg("root")
        .arg("-d")
        .arg(root)
        .arg(image)
        .arg(format!("{size_mib}M"))
        .output()
        .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("cannot run mkfs.ext4: {e}")))?;
    if !output.status.success() {
        return Err(SandboxError::new(
            ErrorCode::Internal,
            format!(
                "mkfs.ext4 failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> SandboxError {
    SandboxError::new(ErrorCode::Internal, format!("export: {error}"))
}
