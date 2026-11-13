//! Exports a session's filesystem as an ext4 image (PLAN.md section 4.11, `export_ext4`) and
//! imports an image back into a session (`import_from_dir`), so a guest can sync its root disk
//! back into the VFS on demand. Both use `mkfs.ext4 -d`'s scratch layout: `export_ext4`
//! writes the tree to a scratch directory and `mkfs.ext4 -d` populates an image from it, and
//! `import_from_dir` reads the tree back out of the image's contents. Neither needs a mount or
//! privilege.
//!
//! Symbolic links are refused on export, as promotion refuses them: an image that dropped a
//! link would not match the session. Device nodes and sockets do not exist in the VFS.

use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{OpenFlags, SessionFs};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::task::spawn_blocking;

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReport {
    pub path: PathBuf,
    pub size_mib: u64,
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
}

/// Reads `image` (an ext4 filesystem produced by `export_ext4` or a guest's root disk) and
/// materializes its files into `fs`, the inverse of `export_ext4`. The image's contents are
/// recovered into a scratch directory, and every directory and file under it is written into
/// the session's VFS, parents before children. The image's root directory counts as the
/// session's `/`, which always exists, so it is not created. Symlinks and other non-regular
/// entries are skipped, as export refuses to carry them into an image.
pub async fn import_from_dir(
    fs: &dyn SessionFs,
    image: &Path,
    scratch: &Path,
) -> Result<ImageReport, SandboxError> {
    let image = image.to_path_buf();
    let scratch = scratch.to_path_buf();
    let dump_scratch = scratch.clone();
    let dump_image = image.clone();
    spawn_blocking(move || {
        let _ = std::fs::remove_dir_all(&dump_scratch);
        std::fs::create_dir_all(&dump_scratch).map_err(io_error)?;
        dump_image_contents(&dump_image, &dump_scratch)
    })
    .await
    .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("import: {e}")))??;
    let mut totals = Totals::default();
    let outcome = walk_into_fs(fs, &scratch, &mut totals).await;
    let result = match outcome {
        Ok(()) => Ok(ImageReport {
            path: image,
            size_mib: totals.bytes / (1024 * 1024),
            files: totals.files,
            directories: totals.directories,
            bytes: totals.bytes,
        }),
        Err(error) => Err(error),
    };
    let cleanup = scratch.clone();
    let _ = spawn_blocking(move || std::fs::remove_dir_all(&cleanup)).await;
    result
}

#[derive(Default)]
struct Totals {
    files: u64,
    directories: u64,
    bytes: u64,
}

/// Writes `fs` to `image` as an ext4 filesystem, labelled `root`. The image is sized from the
/// tree's contents plus room for the filesystem's own metadata, and at least 16 MiB.
pub async fn export_ext4(
    fs: &dyn SessionFs,
    image: &Path,
    scratch: &Path,
) -> Result<ImageReport, SandboxError> {
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

/// Copies every directory and file under `/` into `root`, parents before children.
async fn copy_tree(
    fs: &dyn SessionFs,
    root: &Path,
    totals: &mut Totals,
) -> Result<(), SandboxError> {
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
                    format!(
                        "cannot export the symlink {path}: links are not carried into an image yet"
                    ),
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

/// Reads the image's files out of its ext4 filesystem into `scratch` using `debugfs`'s
/// `rdump` command, which recursively recreates the directory tree without mounting the image.
fn dump_image_contents(image: &Path, scratch: &Path) -> Result<(), SandboxError> {
    let output = Command::new("debugfs")
        .arg("-R")
        .arg(format!("rdump / {}", scratch.display()))
        .arg(image)
        .output()
        .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("cannot run debugfs: {e}")))?;
    if !output.status.success() {
        return Err(SandboxError::new(
            ErrorCode::Internal,
            format!(
                "debugfs rdump failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(())
}

/// Walks the recovered tree (relative paths under the image root) and writes each directory
/// and file into `fs`, parents before children.
async fn walk_into_fs(
    fs: &dyn SessionFs,
    scratch: &Path,
    totals: &mut Totals,
) -> Result<(), SandboxError> {
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let host_dir = scratch.join(&dir);
        let entries = spawn_blocking({
            let host_dir = host_dir.clone();
            move || {
                std::fs::read_dir(&host_dir)
                    .map_err(io_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(io_error)
            }
        })
        .await
        .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("import: {e}")))??;
        let mut entries = entries;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            // `mkfs.ext4` creates an empty `lost+found` directory at the
            // image root; it is filesystem bookkeeping, not session content.
            if dir.as_os_str().is_empty() && entry.file_name() == "lost+found" {
                continue;
            }
            let rel = dir.join(entry.file_name());
            let file_type = entry.file_type().map_err(io_error)?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let vfs_path = format!("/{}", rel.display());
                fs.mkdir(Path::new(&vfs_path), 0o755)
                    .await
                    .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
                totals.directories += 1;
                pending.push(rel);
            } else if file_type.is_file() {
                let vfs_path = format!("/{}", rel.display());
                let file_path = entry.path();
                let data = spawn_blocking({
                    let file_path = file_path.clone();
                    move || std::fs::read(&file_path).map_err(io_error)
                })
                .await
                .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("import: {e}")))??;
                totals.bytes += data.len() as u64;
                totals.files += 1;
                let flags = OpenFlags {
                    write: true,
                    create: true,
                    truncate: true,
                    ..OpenFlags::default()
                };
                let handle = fs
                    .open(Path::new(&vfs_path), flags)
                    .await
                    .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
                let written = fs
                    .write(&handle, 0, &data)
                    .await
                    .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))?;
                if written != data.len() {
                    fs.close(&handle);
                    return Err(SandboxError::new(
                        ErrorCode::Internal,
                        format!("short write to {vfs_path}"),
                    ));
                }
                fs.close(&handle);
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
        .map_err(|e| {
            SandboxError::new(ErrorCode::Internal, format!("cannot run mkfs.ext4: {e}"))
        })?;
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
    SandboxError::new(ErrorCode::Internal, format!("image: {error}"))
}
