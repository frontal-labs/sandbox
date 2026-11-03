//! Ops exposed to scripts. Each op is a host capability, so adding one is a
//! security decision.

use deno_core::{op2, OpState};
use deno_error::JsErrorBox;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs, VfsError};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Convert a VfsError to JsErrorBox.
fn vfs_error_to_js(error: VfsError) -> JsErrorBox {
    use sandbox_vfs::VfsError;
    match error {
        VfsError::NotFound(m) => JsErrorBox::new("NotFound", m),
        VfsError::AlreadyExists(m) => JsErrorBox::new("AlreadyExists", m),
        VfsError::InvalidPath(m) => JsErrorBox::new("InvalidData", m),
        VfsError::PermissionDenied(m) => JsErrorBox::new("PermissionDenied", m),
        VfsError::QuotaExceeded(m) => JsErrorBox::new("ResourceExhausted", m),
        VfsError::Unsupported(m) => JsErrorBox::new("NotSupported", m),
        other => JsErrorBox::new("Error", other.to_string()),
    }
}

/// Filesystem ops for the node: module shim.
pub mod fs {
    use super::*;
    use deno_core::JsBuffer;

    /// Reads a file from the VFS.
    #[op2(async(lazy), fast)]
    #[buffer]
    pub async fn op_fs_read(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
    ) -> Result<Vec<u8>, JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let flags = OpenFlags::default();
        let handle = vfs
            .open(std::path::Path::new(&path), flags)
            .await
            .map_err(vfs_error_to_js)?;
        let mut buf = Vec::new();
        loop {
            let chunk = vfs
                .read(&handle, buf.len() as u64, 65536)
                .await
                .map_err(vfs_error_to_js)?;
            if chunk.is_empty() {
                vfs.close(&handle);
                return Ok(buf);
            }
            buf.extend_from_slice(&chunk);
        }
    }

    /// Writes a file to the VFS.
    #[op2(async(lazy))]
    #[buffer]
    pub async fn op_fs_write(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
        #[buffer] data: JsBuffer,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let flags = OpenFlags {
            read: false,
            write: true,
            create: true,
            truncate: true,
            ..OpenFlags::default()
        };
        let handle = vfs
            .open(std::path::Path::new(&path), flags)
            .await
            .map_err(vfs_error_to_js)?;
        vfs.write(&handle, 0, &data)
            .await
            .map_err(vfs_error_to_js)?;
        vfs.close(&handle);
        Ok(())
    }

    /// Stats a file in the VFS. Returns JSON string.
    #[op2(async(lazy), fast)]
    #[string]
    pub async fn op_fs_stat(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
    ) -> Result<String, JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let stat = vfs
            .stat(std::path::Path::new(&path))
            .await
            .map_err(vfs_error_to_js)?;
        let json = serde_json::json!({
            "isFile": stat.is_file,
            "isDirectory": stat.is_dir,
            "isSymlink": stat.is_symlink,
            "size": stat.size,
            "mode": stat.mode,
            "mtime": stat.modified,
        });
        Ok(json.to_string())
    }

    /// Lists a directory in the VFS. Returns JSON string array.
    #[op2(async(lazy), fast)]
    #[string]
    pub async fn op_fs_readdir(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
    ) -> Result<String, JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let entries = vfs
            .readdir(std::path::Path::new(&path))
            .await
            .map_err(vfs_error_to_js)?;
        let result: Vec<serde_json::Value> = entries
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.name,
                    "isFile": e.is_file,
                    "isDirectory": e.is_dir,
                    "isSymlink": e.is_symlink,
                })
            })
            .collect();
        serde_json::to_string(&result).map_err(JsErrorBox::from_err)
    }

    /// Creates a directory in the VFS.
    #[op2(async(lazy))]
    pub async fn op_fs_mkdir(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
        #[string] mode: Option<String>,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let mode = mode
            .and_then(|m| u32::from_str_radix(&m, 8).ok())
            .unwrap_or(0o755);
        vfs.mkdir(std::path::Path::new(&path), mode)
            .await
            .map_err(vfs_error_to_js)?;
        Ok(())
    }

    /// Removes a file or directory in the VFS.
    #[op2(async(lazy), fast)]
    pub async fn op_fs_rm(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
        recursive: bool,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        vfs.rm(std::path::Path::new(&path), recursive)
            .await
            .map_err(vfs_error_to_js)?;
        Ok(())
    }

    /// Renames a file or directory in the VFS.
    #[op2(async(lazy), fast)]
    pub async fn op_fs_rename(
        state: Rc<RefCell<OpState>>,
        #[string] old_path: String,
        #[string] new_path: String,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        vfs.rename(
            std::path::Path::new(&old_path),
            std::path::Path::new(&new_path),
        )
        .await
        .map_err(vfs_error_to_js)?;
        Ok(())
    }

    /// Creates a symlink in the VFS.
    #[op2(async(lazy), fast)]
    pub async fn op_fs_symlink(
        state: Rc<RefCell<OpState>>,
        #[string] target: String,
        #[string] path: String,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        vfs.symlink(std::path::Path::new(&target), std::path::Path::new(&path))
            .await
            .map_err(vfs_error_to_js)?;
        Ok(())
    }

    /// Reads a symlink in the VFS.
    #[op2(async(lazy), fast)]
    #[string]
    pub async fn op_fs_readlink(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
    ) -> Result<String, JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        let target = vfs
            .readlink(std::path::Path::new(&path))
            .await
            .map_err(vfs_error_to_js)?;
        Ok(target)
    }

    /// Truncates a file in the VFS.
    #[op2(async(lazy), fast)]
    pub async fn op_fs_truncate(
        state: Rc<RefCell<OpState>>,
        #[string] path: String,
        #[bigint] len: u64,
    ) -> Result<(), JsErrorBox> {
        let vfs = state.borrow().borrow::<Arc<Vfs>>().clone();
        vfs.truncate(std::path::Path::new(&path), len)
            .await
            .map_err(vfs_error_to_js)?;
        Ok(())
    }
}

/// Environment ops.
pub mod env {
    use super::*;

    /// Returns the current environment variables as a JSON string.
    #[op2(async(lazy), fast)]
    #[string]
    pub async fn op_sandbox_env(_state: Rc<RefCell<OpState>>) -> Result<String, JsErrorBox> {
        let env: Vec<(String, String)> = std::env::vars().collect();
        serde_json::to_string(&env).map_err(JsErrorBox::from_err)
    }
}

/// Time ops.
pub mod time {
    use super::*;

    /// Returns the current time in milliseconds since epoch.
    #[op2(fast)]
    #[bigint]
    pub fn op_sandbox_now(_state: &mut OpState) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }
}

/// Process ops.
pub mod process {}

/// Stdout/stderr streaming ops.
pub mod stdio {
    use super::*;

    /// Writes to stdout.
    #[op2(fast)]
    pub fn op_stdout_write(state: &mut OpState, #[buffer] data: &[u8]) {
        if let Some(output) = state.try_borrow_mut::<ScriptOutput>() {
            output.push(&String::from_utf8_lossy(data));
        }
    }

    /// Writes to stderr.
    #[op2(fast)]
    pub fn op_stderr_write(state: &mut OpState, #[buffer] data: &[u8]) {
        if let Some(output) = state.try_borrow_mut::<ScriptOutput>() {
            output.push(&String::from_utf8_lossy(data));
        }
    }
}

/// Receives script output as it is kept, in order. See [`ScriptOutput::push`].
pub type StreamObserver = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Output captured for one script run. Stored in `OpState` for the run.
#[derive(Default)]
pub struct ScriptOutput {
    pub stdout: Vec<u8>,
    /// Maximum bytes kept in `stdout`. Anything beyond it is dropped.
    pub limit: usize,
    pub truncated: bool,
    /// Told each piece of output as it is kept. Its pieces concatenate to `stdout`.
    pub observer: Option<StreamObserver>,
}

impl ScriptOutput {
    pub fn push(&mut self, text: &str) {
        let room = self.limit.saturating_sub(self.stdout.len());
        let kept = if text.len() <= room {
            text.as_bytes()
        } else {
            self.truncated = true;
            &text.as_bytes()[..room]
        };
        self.stdout.extend_from_slice(kept);
        if let Some(observer) = &self.observer {
            observer(kept);
        }
    }
}

/// Appends text to the captured stdout. Backs `console.log`.
#[op2(fast)]
pub fn op_sandbox_print(state: &mut OpState, #[string] text: String) {
    if let Some(output) = state.try_borrow_mut::<ScriptOutput>() {
        output.push(&text);
    }
}
