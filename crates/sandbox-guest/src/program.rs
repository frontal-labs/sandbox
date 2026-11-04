//! Programs the guest runs from its own image. A command the shell does not provide runs as
//! a real process, under busybox's `chroot`, in a copy of the session's tree. When the
//! process exits, its changes to the tree are written back to the session's filesystem, so
//! the session keeps the one copy of its files.
//!
//! What a program sees: the session's tree at `/`, minus `/bin`, which holds busybox and its
//! applet links. Only applets busybox provides can run. Symbolic links in the session are not
//! carried into the copy, and the copy is made per command, so large trees cost time.

use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{DirEntry, OpenFlags, SessionFs, Vfs};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;

/// The busybox binary in the guest image. The image's init copies it to `/bin`.
pub const BUSYBOX: &str = "/bin/busybox";
/// Bytes moved per VFS call when copying a file in or out.
const CHUNK: usize = 1024 * 1024;
/// Directory under the guest's real filesystem where per-command copies are made.
const SCRATCH: &str = "/run/sandbox";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// What a program produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `argv` as a real process in a copy of `vfs`, with `cwd`, `env`, and `stdin`, and then
/// writes the tree's changes back to `vfs`.
pub async fn run_program(
    vfs: &Vfs,
    argv: &[String],
    env: &[(String, String)],
    cwd: &str,
    stdin: &[u8],
) -> Result<ProgramOutput, SandboxError> {
    let root = PathBuf::from(SCRATCH).join(format!(
        "run-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let outcome = run_in(vfs, &root, argv, env, cwd, stdin).await;
    // The copy goes whether or not the program succeeded.
    let _ = std::fs::remove_dir_all(&root);
    outcome
}

async fn run_in(
    vfs: &Vfs,
    root: &Path,
    argv: &[String],
    env: &[(String, String)],
    cwd: &str,
    stdin: &[u8],
) -> Result<ProgramOutput, SandboxError> {
    let io = |e: std::io::Error| SandboxError::new(ErrorCode::Internal, format!("guest copy: {e}"));
    std::fs::create_dir_all(root.join("bin")).map_err(io)?;
    std::fs::copy(BUSYBOX, root.join("bin/busybox")).map_err(io)?;
    // The shell that runs the command, and the command's own name, are busybox applets.
    let mut names = vec!["sh".to_string()];
    names.extend(argv.first().filter(|name| !name.contains('/')).cloned());
    for name in names {
        let link = root.join("bin").join(&name);
        if !link.exists() {
            std::os::unix::fs::symlink("busybox", &link).map_err(io)?;
        }
    }
    copy_out(vfs, root).await?;

    // busybox's chroot needs root. The guest's init runs as root, so it has it.
    let script = format!("cd {} && exec \"$@\"", quote(cwd));
    let mut command = tokio::process::Command::new(BUSYBOX);
    command
        .arg("chroot")
        .arg(root)
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .args(argv)
        .env_clear()
        .env("PATH", "/bin:/usr/bin")
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("cannot start {}: {e}", argv[0]),
        )
    })?;
    if let Some(mut input) = child.stdin.take() {
        // A program that never reads its stdin closes the pipe, and that is not an error.
        let _ = input.write_all(stdin).await;
    }
    let output = child.wait_with_output().await.map_err(|e| {
        SandboxError::new(ErrorCode::Internal, format!("waiting for {}: {e}", argv[0]))
    })?;

    copy_back(vfs, root).await?;
    Ok(ProgramOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// Single-quotes a string for the shell that `chroot` starts.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// `/bin` belongs to busybox and its links, so neither the copy nor the write-back touches it.
fn in_system_dir(path: &str) -> bool {
    path == "/bin" || path.starts_with("/bin/")
}

fn vfs_error(error: sandbox_vfs::VfsError) -> SandboxError {
    SandboxError::new(ErrorCode::VfsError, error.to_string())
}

async fn read_all(vfs: &Vfs, path: &str) -> Result<Vec<u8>, SandboxError> {
    let handle = vfs
        .open(Path::new(path), OpenFlags::default())
        .await
        .map_err(vfs_error)?;
    let mut out = Vec::new();
    let mut finished = false;
    while !finished {
        let chunk = vfs
            .read(&handle, out.len() as u64, CHUNK)
            .await
            .map_err(vfs_error)?;
        finished = chunk.len() < CHUNK;
        out.extend_from_slice(&chunk);
    }
    vfs.close(&handle);
    Ok(out)
}

async fn write_all(vfs: &Vfs, path: &str, data: &[u8]) -> Result<(), SandboxError> {
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new(path), flags).await.map_err(vfs_error)?;
    // An empty file is still written once, so it exists afterwards.
    let mut at = 0usize;
    let mut first = true;
    while first || at < data.len() {
        first = false;
        let end = (at + CHUNK).min(data.len());
        vfs.write(&handle, at as u64, &data[at..end])
            .await
            .map_err(vfs_error)?;
        at = end;
    }
    vfs.close(&handle);
    Ok(())
}

/// Copies the session's tree to the real directory `root`.
async fn copy_out(vfs: &Vfs, root: &Path) -> Result<(), SandboxError> {
    let mut pending = vec!["/".to_string()];
    while let Some(dir) = pending.pop() {
        let entries: Vec<DirEntry> = vfs.readdir(Path::new(&dir)).await.map_err(vfs_error)?;
        for entry in entries {
            let path = join(&dir, &entry.name);
            if in_system_dir(&path) {
                continue;
            }
            let host = root.join(path.trim_start_matches('/'));
            if entry.is_dir {
                std::fs::create_dir_all(&host).map_err(io_error)?;
                pending.push(path);
            } else if entry.is_file {
                let data = read_all(vfs, &path).await?;
                std::fs::write(&host, data).map_err(io_error)?;
            }
        }
    }
    Ok(())
}

/// Brings the session's tree in line with the real directory `root`: changed and new files
/// are written, and paths the program removed are removed.
async fn copy_back(vfs: &Vfs, root: &Path) -> Result<(), SandboxError> {
    let mut host_paths = BTreeSet::new();
    let mut pending = vec![("/".to_string(), root.to_path_buf())];
    while let Some((dir, host_dir)) = pending.pop() {
        for entry in std::fs::read_dir(&host_dir).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = join(&dir, &name);
            if in_system_dir(&path) {
                continue;
            }
            let host = entry.path();
            let kind = entry.file_type().map_err(io_error)?;
            host_paths.insert(path.clone());
            if kind.is_dir() {
                if vfs.stat(Path::new(&path)).await.is_err() {
                    vfs.mkdir(Path::new(&path), 0o755)
                        .await
                        .map_err(vfs_error)?;
                }
                pending.push((path, host));
            } else if kind.is_file() {
                let data = std::fs::read(&host).map_err(io_error)?;
                let same = match vfs.stat(Path::new(&path)).await {
                    Ok(stat) if stat.is_file && stat.size == data.len() as u64 => {
                        read_all(vfs, &path).await? == data
                    }
                    _ => false,
                };
                if !same {
                    write_all(vfs, &path, &data).await?;
                }
            }
        }
    }

    // Paths the program removed. Children sort after their parents, so the reverse order
    // removes children first and a directory is empty when its turn comes.
    let mut gone = Vec::new();
    let mut pending = vec!["/".to_string()];
    while let Some(dir) = pending.pop() {
        for entry in vfs.readdir(Path::new(&dir)).await.map_err(vfs_error)? {
            let path = join(&dir, &entry.name);
            if in_system_dir(&path) {
                continue;
            }
            if !host_paths.contains(&path) {
                gone.push(path.clone());
            }
            if entry.is_dir {
                pending.push(path);
            }
        }
    }
    gone.sort();
    for path in gone.iter().rev() {
        // A directory removed with its children is already gone, and that is not an error.
        let _ = vfs.rm(Path::new(path), true).await;
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> SandboxError {
    SandboxError::new(ErrorCode::Internal, format!("guest copy: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_quotes_and_spaces() {
        assert_eq!(quote("/a b"), "'/a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn the_system_directory_is_recognised() {
        assert!(in_system_dir("/bin"));
        assert!(in_system_dir("/bin/busybox"));
        assert!(!in_system_dir("/binary/x"));
        assert!(!in_system_dir("/work"));
    }
}
