//! Durable server state. See ADR 0008.
//!
//! `state.json` in the data directory holds the API key digests, the snapshot
//! records, and the sleeping sessions. It is rewritten whole, to a temporary file
//! that is then renamed over it, so a crash leaves either the old file or the
//! new one. Blobs live in `blobs/`, in the disk store.

use sandbox_cas::DiskStore;
use sandbox_session::install_blob_store;
use sandbox_types::hash::ContentHash;
use sandbox_types::{SandboxError, SessionId, SessionState, SnapshotId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const FILE: &str = "state.json";
const VERSION: u32 = 1;

/// The data directory this process installed its blob store for. A process has
/// one blob store, so reopening the same directory is fine and another one is not.
static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    pub keys: Vec<SavedKey>,
    pub snapshots: Vec<SavedSnapshot>,
    pub sleeping: Vec<SavedSleep>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedKey {
    pub id: String,
    /// Hex BLAKE3 digest of the secret. The secret itself is never stored.
    pub digest: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSnapshot {
    pub id: SnapshotId,
    pub owner: String,
    pub session: SessionId,
    pub root: ContentHash,
    #[serde(with = "sandbox_types::rfc3339")]
    pub created_at: SystemTime,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSleep {
    pub owner: String,
    pub state: SessionState,
    pub sleep_root: ContentHash,
    /// The session's trace in the ADR 0007 format.
    pub trace: String,
    pub trace_complete: bool,
}

/// The state file in one data directory.
pub struct Store {
    path: PathBuf,
    /// Serializes writes, so two saves cannot interleave their renames.
    write: tokio::sync::Mutex<()>,
}

impl Store {
    /// Opens the data directory, installs its blob store, and reads the saved
    /// state, which is empty for a new directory.
    pub fn open(dir: &Path) -> Result<(Self, Saved), SandboxError> {
        let to_error = |e: std::io::Error| {
            SandboxError::internal(format!("cannot create {}: {e}", dir.display()))
        };
        create_private_dir(dir).map_err(to_error)?;
        create_private_dir(&dir.join("blobs")).map_err(to_error)?;
        install_blob_store_once(dir)?;

        let path = dir.join(FILE);
        let saved = match std::fs::read(&path) {
            Ok(bytes) => {
                let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| {
                    SandboxError::internal(format!("cannot read {}: {e}", path.display()))
                })?;
                if saved.version != VERSION {
                    return Err(SandboxError::internal(format!(
                        "{} has state version {}; this build reads version {VERSION}",
                        path.display(),
                        saved.version
                    )));
                }
                saved
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved {
                version: VERSION,
                ..Saved::default()
            },
            Err(e) => {
                return Err(SandboxError::internal(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };
        Ok((
            Self {
                path,
                write: tokio::sync::Mutex::new(()),
            },
            saved,
        ))
    }

    /// Captures the state with `capture` and writes it, under one lock. Two
    /// saves therefore cannot write their captures out of order, so the file
    /// always holds the newest state.
    pub async fn save<F, Fut>(&self, capture: F) -> Result<(), SandboxError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Saved>,
    {
        let _guard = self.write.lock().await;
        let saved = capture().await;
        self.write_locked(&saved).await
    }

    /// Writes the state atomically: a temporary file, synced, then renamed. The
    /// caller holds the write lock.
    async fn write_locked(&self, saved: &Saved) -> Result<(), SandboxError> {
        let bytes = serde_json::to_vec_pretty(saved)
            .map_err(|e| SandboxError::internal(format!("cannot encode state: {e}")))?;
        // A unique name, created exclusively, so a file planted at a predictable
        // path cannot be followed or overwritten.
        let tmp = self.path.with_file_name(format!(
            "{FILE}.{}.{}.tmp",
            std::process::id(),
            sandbox_types::SessionId::new()
        ));
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)
        })
        .await
        .map_err(|e| SandboxError::internal(format!("state write task failed: {e}")))?
        .map_err(|e| SandboxError::internal(format!("cannot save state: {e}")))
    }
}

/// Creates `path` and its parents. A directory this call creates is owner-only on
/// Unix, because the state holds key digests and snapshot roots. A directory that
/// already exists keeps its permissions.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    // The mode is applied when the directory is created, so there is no window in
    // which it exists with default permissions. An existing directory is left alone.
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Installs the disk store for `dir` once per process. Reopening the same
/// directory is allowed. A different directory is refused.
fn install_blob_store_once(dir: &Path) -> Result<(), SandboxError> {
    let mut installed = INSTALLED.lock().unwrap_or_else(|p| p.into_inner());
    match installed.as_deref() {
        Some(existing) if existing == dir => Ok(()),
        Some(existing) => Err(SandboxError::new(
            sandbox_types::ErrorCode::InvalidState,
            format!(
                "this process already uses {} for its blobs; cannot switch to {}",
                existing.display(),
                dir.display()
            ),
        )),
        None => {
            let blobs = DiskStore::new(dir.join("blobs"))
                .map_err(|e| SandboxError::internal(format!("cannot open the blob store: {e}")))?;
            install_blob_store(Arc::new(blobs))?;
            *installed = Some(dir.to_path_buf());
            Ok(())
        }
    }
}
