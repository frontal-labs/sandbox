//! Durable server state. See ADR 0008.
//!
//! The state lives in the data directory as a base file and a log (ADR 0016).
//! `state.json` is a full snapshot of the API key digests, the snapshot records,
//! and the sessions. `state.<epoch>.log` holds, as JSON lines, the entities that
//! changed since that snapshot. Each save appends only what changed, and a
//! compaction writes a new snapshot when the log outgrows it. Blobs live in
//! `blobs/`, in the disk store.

use sandbox_cas::DiskStore;
use sandbox_session::install_blob_store;
use sandbox_types::hash::ContentHash;
use sandbox_types::{CheckpointPolicy, SandboxError, SessionId, SessionState, SnapshotId};
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
    /// Which log holds the changes made since this file was written (ADR 0016).
    /// Files written before the log existed have none, so they are epoch 0.
    #[serde(default)]
    pub log_epoch: u64,
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
    /// The limits the session was created with (ADR 0014). Older state has none,
    /// so it takes the defaults.
    #[serde(default)]
    pub limits: sandbox_types::policy::ResourceLimits,
    /// The session's trace in the ADR 0007 format.
    pub trace: String,
    pub trace_complete: bool,
    /// When the session checkpoints. Older state has none, so it takes the default.
    #[serde(default)]
    pub checkpoint_policy: CheckpointPolicy,
    #[serde(default = "default_checkpoint_retain")]
    pub checkpoint_retain: u32,
    /// The pinned checkpoint roots, oldest first (IMAGE.md §4.5).
    #[serde(default)]
    pub roots: Vec<CheckpointRoot>,
    /// Outputs of runs that did not fit in the trace.
    #[serde(default)]
    pub spilled: Vec<SpilledRun>,
}

fn default_checkpoint_retain() -> u32 {
    sandbox_types::session::DEFAULT_CHECKPOINT_RETAIN
}

/// The outputs of a run that did not fit in the trace. They are kept as blobs, so the
/// full stream stays readable at `trace/exec/<seq>.out` and `.err` (IMAGE.md §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpilledRun {
    pub seq: u64,
    pub stdout: ContentHash,
    pub stderr: ContentHash,
    pub bytes: u64,
}

/// A pinned checkpoint: a committed root and its logical size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRoot {
    pub root: ContentHash,
    pub bytes: u64,
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
    /// The limits of the session the snapshot was taken from (ADR 0014).
    #[serde(default)]
    pub limits: sandbox_types::policy::ResourceLimits,
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

/// Appends smaller than this never trigger a compaction, so a small state is not
/// rewritten every few changes.
const COMPACT_FLOOR: u64 = 1024 * 1024;

fn log_name(epoch: u64) -> String {
    format!("state.{epoch}.log")
}

/// One saved entity, by its key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Entity {
    Key(String),
    Snapshot(SnapshotId),
    Session(SessionId),
}

/// Every saved entity, with its canonical JSON and that JSON's hash. Canonical
/// JSON sorts object keys, so an entity that did not change hashes the same on
/// every save.
type Canonical = HashMap<Entity, (ContentHash, serde_json::Value)>;

/// A logged change, as read back on open. The log writes these as
/// `{"op": ..., ...}` lines.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Record {
    PutKey { value: SavedKey },
    DelKey { id: String },
    PutSnapshot { value: SavedSnapshot },
    DelSnapshot { id: SnapshotId },
    PutSession { value: SavedSession },
    DelSession { id: SessionId },
}

/// The saved state keyed by entity, the working form while the log is replayed.
#[derive(Default)]
struct Entities {
    keys: HashMap<String, SavedKey>,
    snapshots: HashMap<SnapshotId, SavedSnapshot>,
    sessions: HashMap<SessionId, SavedSession>,
    sleeping: Vec<SavedSleep>,
}

impl Entities {
    fn from_saved(saved: Saved) -> Self {
        Self {
            keys: saved.keys.into_iter().map(|k| (k.id.clone(), k)).collect(),
            snapshots: saved.snapshots.into_iter().map(|s| (s.id, s)).collect(),
            sessions: saved
                .sessions
                .into_iter()
                .map(|s| (s.state.id, s))
                .collect(),
            sleeping: saved.sleeping,
        }
    }

    fn apply(&mut self, record: Record) {
        match record {
            Record::PutKey { value } => {
                self.keys.insert(value.id.clone(), value);
            }
            Record::DelKey { id } => {
                self.keys.remove(&id);
            }
            Record::PutSnapshot { value } => {
                self.snapshots.insert(value.id, value);
            }
            Record::DelSnapshot { id } => {
                self.snapshots.remove(&id);
            }
            Record::PutSession { value } => {
                self.sessions.insert(value.state.id, value);
            }
            Record::DelSession { id } => {
                self.sessions.remove(&id);
            }
        }
    }

    fn into_saved(self, epoch: u64) -> Saved {
        Saved {
            version: VERSION,
            keys: self.keys.into_values().collect(),
            snapshots: self.snapshots.into_values().collect(),
            sleeping: self.sleeping,
            sessions: self.sessions.into_values().collect(),
            log_epoch: epoch,
        }
    }
}

/// What the store has written so far, with the epoch and sizes it writes to.
struct LogState {
    epoch: u64,
    /// Bytes in the current base file.
    base_bytes: u64,
    /// Bytes in the current log.
    log_bytes: u64,
    /// Every entity as the store last left it on disk.
    written: Canonical,
}

/// The state in one data directory. See the module comment and ADR 0016.
pub struct Store {
    dir: PathBuf,
    /// Serializes saves, so two saves cannot interleave their appends or
    /// compactions.
    state: tokio::sync::Mutex<LogState>,
}

impl Store {
    /// Opens the data directory, installs its blob store, and reads the saved
    /// state: the base file, then the log for its epoch. A new directory has an
    /// empty state.
    pub fn open(dir: &Path) -> Result<(Self, Saved), SandboxError> {
        prepare_blobs(dir)?;
        Self::load(dir)
    }

    /// Reads the saved state in `dir` without touching the blob store. `open`
    /// calls this after it installs the store.
    pub(crate) fn load(dir: &Path) -> Result<(Self, Saved), SandboxError> {
        let (base, base_bytes) = read_base(&dir.join(FILE))?;
        let epoch = base.log_epoch;
        let mut entities = Entities::from_saved(base);
        let log_bytes = replay(&dir.join(log_name(epoch)), &mut entities)?;
        let saved = entities.into_saved(epoch);
        let written = canonical(&saved)?;
        Ok((
            Self {
                dir: dir.to_path_buf(),
                state: tokio::sync::Mutex::new(LogState {
                    epoch,
                    base_bytes,
                    log_bytes,
                    written,
                }),
            },
            saved,
        ))
    }

    /// Captures the state with `capture`, and writes what changed since the last
    /// save. Saves are serialized, so the log is written in order and the
    /// newest state wins on replay.
    pub async fn save<F, Fut>(&self, capture: F) -> Result<(), SandboxError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Saved, SandboxError>>,
    {
        let mut state = self.state.lock().await;
        let saved = capture().await?;
        let current = canonical(&saved)?;

        let mut lines = Vec::new();
        for (entity, (hash, value)) in &current {
            if state.written.get(entity).map(|(old, _)| old) != Some(hash) {
                append_line(&mut lines, put_record(entity, value))?;
            }
        }
        for entity in state.written.keys() {
            if !current.contains_key(entity) {
                append_line(&mut lines, del_record(entity)?)?;
            }
        }
        if lines.is_empty() {
            return Ok(());
        }

        let grown = state.log_bytes + lines.len() as u64;
        if grown > COMPACT_FLOOR.max(state.base_bytes) {
            self.compact(&mut state, saved, current).await
        } else {
            self.append(&mut state, lines).await?;
            state.written = current;
            Ok(())
        }
    }

    /// Appends `bytes` to the current log and syncs it.
    async fn append(&self, state: &mut LogState, bytes: Vec<u8>) -> Result<(), SandboxError> {
        let path = self.dir.join(log_name(state.epoch));
        let len = bytes.len() as u64;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path)?;
            file.write_all(&bytes)?;
            file.sync_data()
        })
        .await
        .map_err(|e| SandboxError::internal(format!("state write task failed: {e}")))?
        .map_err(|e| SandboxError::internal(format!("cannot append to the state log: {e}")))?;
        state.log_bytes += len;
        Ok(())
    }

    /// Writes `saved` as the base file for the next epoch, starts an empty log for
    /// it, and removes the old log. A crash at any step leaves a readable store: the
    /// base names its log, and a log from another epoch is never replayed.
    async fn compact(
        &self,
        state: &mut LogState,
        mut saved: Saved,
        current: Canonical,
    ) -> Result<(), SandboxError> {
        let next = state.epoch + 1;
        saved.version = VERSION;
        saved.log_epoch = next;
        let bytes = serde_json::to_vec_pretty(&saved)
            .map_err(|e| SandboxError::internal(format!("cannot encode state: {e}")))?;
        let base_len = bytes.len() as u64;
        write_file_atomic(self.dir.join(FILE), bytes).await?;
        create_empty(self.dir.join(log_name(next))).await?;

        // The old log is no longer read, so a failure to remove it only leaves a
        // file behind. It is named after an epoch that is no longer current.
        let _ = tokio::fs::remove_file(self.dir.join(log_name(state.epoch))).await;

        state.epoch = next;
        state.base_bytes = base_len;
        state.log_bytes = 0;
        state.written = current;
        Ok(())
    }
}

/// Reads the base file. A missing file is an empty state. Returns the state and
/// the file's size.
fn read_base(path: &Path) -> Result<(Saved, u64), SandboxError> {
    match std::fs::read(path) {
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
            Ok((saved, bytes.len() as u64))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((
            Saved {
                version: VERSION,
                ..Saved::default()
            },
            0,
        )),
        Err(e) => Err(SandboxError::internal(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

/// Replays the log at `path` into `entities`, and returns its complete size. A
/// final line without its newline is a write that a crash cut short. It is dropped,
/// and the file is truncated, so the next append starts on a clean line. A damaged
/// line anywhere else is an error, because it cannot be a torn write.
fn replay(path: &Path, entities: &mut Entities) -> Result<u64, SandboxError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(SandboxError::internal(format!(
                "cannot read {}: {e}",
                path.display()
            )))
        }
    };
    let complete = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |last| last + 1);
    for (index, line) in bytes[..complete]
        .split(|b| *b == b'\n')
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let record: Record = serde_json::from_slice(line).map_err(|e| {
            SandboxError::internal(format!(
                "{} is damaged at record {}: {e}",
                path.display(),
                index + 1
            ))
        })?;
        entities.apply(record);
    }
    if complete < bytes.len() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| {
                SandboxError::internal(format!("cannot truncate {}: {e}", path.display()))
            })?;
        file.set_len(complete as u64)
            .and_then(|()| file.sync_all())
            .map_err(|e| {
                SandboxError::internal(format!("cannot truncate {}: {e}", path.display()))
            })?;
    }
    Ok(complete as u64)
}

/// Every saved entity of `saved`, with its canonical JSON and hash.
fn canonical(saved: &Saved) -> Result<Canonical, SandboxError> {
    let mut out = Canonical::new();
    for key in &saved.keys {
        insert_canonical(&mut out, Entity::Key(key.id.clone()), key)?;
    }
    for snapshot in &saved.snapshots {
        insert_canonical(&mut out, Entity::Snapshot(snapshot.id), snapshot)?;
    }
    for session in &saved.sessions {
        insert_canonical(&mut out, Entity::Session(session.state.id), session)?;
    }
    Ok(out)
}

fn insert_canonical<T: Serialize>(
    out: &mut Canonical,
    entity: Entity,
    value: &T,
) -> Result<(), SandboxError> {
    let value = serde_json::to_value(value)
        .map_err(|e| SandboxError::internal(format!("cannot encode state: {e}")))?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|e| SandboxError::internal(format!("cannot encode state: {e}")))?;
    out.insert(entity, (ContentHash::new(&bytes), value));
    Ok(())
}

/// The log line that stores an entity.
fn put_record(entity: &Entity, value: &serde_json::Value) -> serde_json::Value {
    let (op, field) = match entity {
        Entity::Key(_) => ("put_key", "value"),
        Entity::Snapshot(_) => ("put_snapshot", "value"),
        Entity::Session(_) => ("put_session", "value"),
    };
    serde_json::json!({ "op": op, field: value })
}

/// The log line that removes an entity.
fn del_record(entity: &Entity) -> Result<serde_json::Value, SandboxError> {
    let encode = |e: serde_json::Error| SandboxError::internal(format!("cannot encode state: {e}"));
    Ok(match entity {
        Entity::Key(id) => serde_json::json!({ "op": "del_key", "id": id }),
        Entity::Snapshot(id) => {
            serde_json::json!({ "op": "del_snapshot", "id": serde_json::to_value(id).map_err(encode)? })
        }
        Entity::Session(id) => {
            serde_json::json!({ "op": "del_session", "id": serde_json::to_value(id).map_err(encode)? })
        }
    })
}

fn append_line(out: &mut Vec<u8>, record: serde_json::Value) -> Result<(), SandboxError> {
    let bytes = serde_json::to_vec(&record)
        .map_err(|e| SandboxError::internal(format!("cannot encode state: {e}")))?;
    out.extend_from_slice(&bytes);
    out.push(b'\n');
    Ok(())
}

/// Writes `bytes` to `path` atomically: a temporary file, synced, then renamed.
async fn write_file_atomic(path: PathBuf, bytes: Vec<u8>) -> Result<(), SandboxError> {
    // A unique name, created exclusively, so a file planted at a predictable
    // path cannot be followed or overwritten.
    let tmp = path.with_file_name(format!(
        "{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id(),
        sandbox_types::SessionId::new()
    ));
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

/// Creates an empty file at `path`, replacing any file already there.
async fn create_empty(path: PathBuf) -> Result<(), SandboxError> {
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?.sync_all()
    })
    .await
    .map_err(|e| SandboxError::internal(format!("state write task failed: {e}")))?
    .map_err(|e| SandboxError::internal(format!("cannot create the state log: {e}")))
}

/// Creates `path` and its parents. A directory this call creates is owner-only on
/// Unix, because the state holds key digests and snapshot roots. A directory that
/// already exists keeps its permissions.
/// Creates the data directory and its blob directory, and installs the blob store
/// for this process. Both durable backends call this first, because the blobs
/// stay in the directory whichever backend holds the metadata.
pub(crate) fn prepare_blobs(dir: &Path) -> Result<(), SandboxError> {
    let to_error =
        |e: std::io::Error| SandboxError::internal(format!("cannot create {}: {e}", dir.display()));
    create_private_dir(dir).map_err(to_error)?;
    create_private_dir(&dir.join("blobs")).map_err(to_error)?;
    install_blob_store_once(dir)
}

/// Where the durable state lives: a data directory on this node, or the Postgres
/// database that several nodes share (ADR 0016, PLAN §4.8).
pub enum Persistence {
    Files(Store),
    #[cfg(feature = "clustered")]
    Clustered(crate::pg_store::PgStore),
}

impl Persistence {
    /// Captures the state with `capture` and writes it. See [`Store::save`] and
    /// [`crate::pg_store::PgStore::save`].
    pub async fn save<F, Fut>(&self, capture: F) -> Result<(), SandboxError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Saved, SandboxError>>,
    {
        match self {
            Self::Files(store) => store.save(capture).await,
            #[cfg(feature = "clustered")]
            Self::Clustered(store) => store.save(capture).await,
        }
    }
}

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
            allow_vm: false,
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
            limits: Default::default(),
            trace: String::new(),
            trace_complete: true,
            checkpoint_policy: Default::default(),
            checkpoint_retain: 50,
            roots: Vec::new(),
            spilled: Vec::new(),
        };
        let saved = Saved {
            version: VERSION,
            keys: Vec::new(),
            snapshots: Vec::new(),
            sleeping: Vec::new(),
            sessions: vec![session],
            log_epoch: 0,
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

    fn saved_session(id: SessionId, label: &str) -> SavedSession {
        // A fixed time, so a session that did not change saves the same bytes.
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        SavedSession {
            owner: "key".to_string(),
            state: SessionState {
                id,
                tenant_id: sandbox_types::TenantId::nil(),
                tier: sandbox_types::Tier::Isolate,
                allow_vm: false,
                status: sandbox_types::SessionStatus::Running,
                config_hash: ContentHash::nil(),
                vfs_root: None,
                created_at: now,
                updated_at: now,
                labels: HashMap::from([("name".to_string(), label.to_string())]),
            },
            root: ContentHash::nil(),
            cwd: "/work".to_string(),
            env: HashMap::new(),
            sleeping: false,
            limits: Default::default(),
            trace: String::new(),
            trace_complete: true,
            checkpoint_policy: Default::default(),
            checkpoint_retain: 50,
            roots: Vec::new(),
            spilled: Vec::new(),
        }
    }

    fn state_of(sessions: Vec<SavedSession>) -> Saved {
        Saved {
            version: VERSION,
            keys: vec![SavedKey {
                id: "k1".to_string(),
                digest: "digest".to_string(),
            }],
            snapshots: Vec::new(),
            sleeping: Vec::new(),
            sessions,
            log_epoch: 0,
        }
    }

    async fn save(store: &Store, saved: Saved) {
        store.save(move || async move { Ok(saved) }).await.unwrap();
    }

    fn label_of(session: &SavedSession) -> &str {
        &session.state.labels["name"]
    }

    #[tokio::test]
    async fn saves_append_only_what_changed() {
        let d = dir("append");
        let (store, _) = Store::load(&d).unwrap();
        let (a, b) = (SessionId::new(), SessionId::new());
        save(
            &store,
            state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
        )
        .await;
        let log = d.join(log_name(0));
        let first = std::fs::metadata(&log).unwrap().len();

        // Nothing changed, so nothing is appended.
        save(
            &store,
            state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
        )
        .await;
        assert_eq!(std::fs::metadata(&log).unwrap().len(), first);

        // One session changed, so one record is appended, and it is for that session.
        save(
            &store,
            state_of(vec![saved_session(a, "changed"), saved_session(b, "b")]),
        )
        .await;
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            4,
            "a key and two sessions, then one more session"
        );
        assert!(lines[3].contains("put_session") && lines[3].contains("changed"));
        assert!(!d.join(FILE).exists(), "no base file until a compaction");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn replay_restores_the_last_state_including_deletions() {
        let d = dir("replay");
        let (a, b) = (SessionId::new(), SessionId::new());
        {
            let (store, _) = Store::load(&d).unwrap();
            save(
                &store,
                state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
            )
            .await;
            save(&store, state_of(vec![saved_session(a, "changed")])).await;
        }
        let (_, saved) = Store::load(&d).unwrap();
        assert_eq!(saved.sessions.len(), 1, "the removed session stays removed");
        assert_eq!(saved.sessions[0].state.id, a);
        assert_eq!(label_of(&saved.sessions[0]), "changed");
        assert_eq!(saved.keys.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_torn_final_record_is_dropped_and_truncated() {
        let d = dir("torn");
        let a = SessionId::new();
        {
            let (store, _) = Store::load(&d).unwrap();
            save(&store, state_of(vec![saved_session(a, "a")])).await;
        }
        let log = d.join(log_name(0));
        let complete = std::fs::metadata(&log).unwrap().len();
        // A crash mid-append leaves a record with no newline.
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(&mut file, br#"{"op":"put_sess"#).unwrap();
        drop(file);

        let (store, saved) = Store::load(&d).unwrap();
        assert_eq!(saved.sessions.len(), 1);
        assert_eq!(
            std::fs::metadata(&log).unwrap().len(),
            complete,
            "truncated to the last record"
        );

        // The next save appends on a clean line, and it replays.
        save(&store, state_of(vec![saved_session(a, "after")])).await;
        let (_, saved) = Store::load(&d).unwrap();
        assert_eq!(label_of(&saved.sessions[0]), "after");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn compaction_writes_a_new_base_and_starts_a_new_log() {
        let d = dir("compact");
        let a = SessionId::new();
        let big = "x".repeat(COMPACT_FLOOR as usize);
        {
            let (store, _) = Store::load(&d).unwrap();
            save(&store, state_of(vec![saved_session(a, &big)])).await;
        }
        // The first save was larger than the floor, so it compacted.
        assert!(d.join(FILE).exists(), "a base file was written");
        assert!(!d.join(log_name(0)).exists(), "the old log is gone");
        assert_eq!(std::fs::metadata(d.join(log_name(1))).unwrap().len(), 0);
        let (store, saved) = Store::load(&d).unwrap();
        assert_eq!(saved.log_epoch, 1);
        assert_eq!(label_of(&saved.sessions[0]).len(), big.len());

        // A later change goes to the new log, and both halves replay.
        save(&store, state_of(vec![saved_session(a, "small")])).await;
        assert!(std::fs::metadata(d.join(log_name(1))).unwrap().len() > 0);
        let (_, saved) = Store::load(&d).unwrap();
        assert_eq!(label_of(&saved.sessions[0]), "small");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_damaged_record_before_the_end_is_refused() {
        let d = dir("damaged");
        let (a, b) = (SessionId::new(), SessionId::new());
        {
            let (store, _) = Store::load(&d).unwrap();
            save(
                &store,
                state_of(vec![saved_session(a, "a"), saved_session(b, "b")]),
            )
            .await;
        }
        let log = d.join(log_name(0));
        let text = std::fs::read_to_string(&log).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        lines[1] = "not a record";
        std::fs::write(&log, lines.join("\n") + "\n").unwrap();
        let err = Store::load(&d)
            .err()
            .expect("a damaged record must be refused");
        assert!(err.message.contains("damaged"), "{}", err.message);
        let _ = std::fs::remove_dir_all(&d);
    }
}
