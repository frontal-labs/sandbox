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
/// Version 2 saves every session with its checkpoint (ADR 0010). Version 1 files
/// still load: their sleeping sessions are read from `sleeping`.
pub const VERSION: u32 = 2;
const READABLE: &[u32] = &[1, 2];

/// The data directory this process installed its blob store for. A process has
/// one blob store, so reopening the same directory is fine and another one is not.
static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    pub keys: Vec<SavedKey>,
    pub snapshots: Vec<SavedSnapshot>,
    /// Version 1 sleeping sessions, read for compatibility and never written.
    #[serde(default)]
    pub sleeping: Vec<SavedSleep>,
    #[serde(default)]
    pub sessions: Vec<SavedSession>,
}

/// A session saved with its checkpoint, so a restart brings it back. Sleeping
/// sessions are saved too, with `sleeping` set. ADR 0008 and ADR 0010.
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub owner: String,
    pub state: SessionState,
    /// The committed filesystem root.
    pub root: ContentHash,
    #[serde(default = "root_dir")]
    pub cwd: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub sleeping: bool,
    /// The session's trace in the ADR 0007 format.
    pub trace: String,
    pub trace_complete: bool,
}

fn root_dir() -> String {
    "/".to_string()
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
                if !READABLE.contains(&saved.version) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sandbox-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn version_one_state_still_loads_its_sleeping_sessions() {
        let v1 = r#"{"version":1,"keys":[],"snapshots":[],"sleeping":[]}"#;
        let saved: Saved = serde_json::from_str(v1).unwrap();
        assert_eq!(saved.version, 1);
        assert!(saved.sessions.is_empty());
        assert!(READABLE.contains(&saved.version));
    }

    #[test]
    fn a_version_two_session_keeps_its_shell_state() {
        let state = SessionState {
            id: SessionId::new(),
            tenant_id: sandbox_types::TenantId::nil(),
            tier: sandbox_types::Tier::Isolate,
            status: sandbox_types::SessionStatus::Running,
            config_hash: ContentHash::nil(),
            vfs_root: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            labels: HashMap::new(),
        };
        let session = SavedSession {
            owner: "key".to_string(),
            state,
            root: ContentHash::nil(),
            cwd: "/work".to_string(),
            env: HashMap::from([("GREETING".to_string(), "hello".to_string())]),
            sleeping: false,
            trace: String::new(),
            trace_complete: true,
        };
        let saved = Saved {
            version: VERSION,
            keys: Vec::new(),
            snapshots: Vec::new(),
            sleeping: Vec::new(),
            sessions: vec![session],
        };
        let text = serde_json::to_string(&saved).unwrap();
        let back: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(back.sessions[0].cwd, "/work");
        assert_eq!(back.sessions[0].env["GREETING"], "hello");
    }

    #[test]
    fn an_unknown_state_version_is_refused() {
        let d = dir("version");
        std::fs::write(d.join(FILE), r#"{"version":9,"keys":[],"snapshots":[]}"#).unwrap();
        let err = Store::open(&d).err().expect("version 9 must be refused");
        assert!(err.message.contains("version 9"), "{}", err.message);
        let _ = std::fs::remove_dir_all(&d);
    }
}
