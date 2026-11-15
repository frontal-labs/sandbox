//! Records kept one file per ID in a directory of the data directory. Parked sleeping sessions
//! and snapshot records use it: a record is read back only when a request needs it, so the
//! server keeps no copy in memory. Each record is written atomically, and replacing a record
//! replaces its file.

use crate::persist::{create_private_dir, write_file_atomic, SavedSession, SavedSnapshot};
use sandbox_types::{SandboxError, SessionId, SnapshotId};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Display;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// A directory holding one JSON file per record, named `<id>.json`.
pub struct JsonDir<I, T> {
    dir: PathBuf,
    _records: PhantomData<fn() -> (I, T)>,
}

impl<I, T> Clone for JsonDir<I, T> {
    fn clone(&self) -> Self {
        Self {
            dir: self.dir.clone(),
            _records: PhantomData,
        }
    }
}

impl<I, T> JsonDir<I, T>
where
    I: Display + FromStr + Copy,
    T: Serialize + DeserializeOwned,
{
    /// Opens the directory, creating it if needed.
    pub(crate) fn open(dir: &Path) -> Result<Self, SandboxError> {
        create_private_dir(dir)
            .map_err(|e| SandboxError::internal(format!("cannot create {}: {e}", dir.display())))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            _records: PhantomData,
        })
    }

    pub fn path(&self, id: I) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Writes a record, replacing any earlier one with the same ID.
    pub async fn put(&self, id: I, record: &T) -> Result<(), SandboxError> {
        let bytes = serde_json::to_vec(record)
            .map_err(|e| SandboxError::internal(format!("cannot encode a record: {e}")))?;
        write_file_atomic(self.path(id), bytes).await
    }

    /// Reads a record, or `None` when there is none with this ID.
    pub async fn get(&self, id: I) -> Result<Option<T>, SandboxError> {
        let bytes = match tokio::fs::read(self.path(id)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SandboxError::internal(format!("cannot read a record: {e}"))),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| SandboxError::internal(format!("a saved record is damaged: {e}")))
    }

    /// Removes a record. Returns whether there was one.
    pub async fn remove(&self, id: I) -> Result<bool, SandboxError> {
        match tokio::fs::remove_file(self.path(id)).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(SandboxError::internal(format!(
                "cannot remove a record: {e}"
            ))),
        }
    }

    /// Every ID with a record. The records are not read. Files whose names are not IDs, and
    /// temporary files left by an interrupted write, are skipped.
    pub async fn ids(&self) -> Result<Vec<I>, SandboxError> {
        let list_error =
            |e: std::io::Error| SandboxError::internal(format!("cannot list records: {e}"));
        let mut entries = tokio::fs::read_dir(&self.dir).await.map_err(list_error)?;
        let mut ids = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(list_error)? {
            let name = entry.file_name();
            let id = name
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|stem| stem.parse::<I>().ok());
            if let Some(id) = id {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

/// The sleeping sessions of a durable server, one record each.
pub(crate) type ParkedStore = JsonDir<SessionId, SavedSession>;

/// The snapshot records of a durable server: one file per record in `snapshots/`, and an empty
/// link file per snapshot in `snapshots/by-session/<session>/`, so a session's snapshots are
/// found without reading every record. The server keeps neither in memory.
#[derive(Clone)]
pub(crate) struct SnapshotFiles {
    records: JsonDir<SnapshotId, SavedSnapshot>,
    links: PathBuf,
}

impl SnapshotFiles {
    /// Opens the snapshot directories in `data_dir`, creating them if needed.
    pub(crate) fn open(data_dir: &Path) -> Result<Self, SandboxError> {
        let records = JsonDir::open(&data_dir.join("snapshots"))?;
        let links = data_dir.join("snapshots").join("by-session");
        create_private_dir(&links).map_err(|e| {
            SandboxError::internal(format!("cannot create {}: {e}", links.display()))
        })?;
        Ok(Self { records, links })
    }

    /// Writes a snapshot's record and its session link.
    pub(crate) async fn put(&self, record: &SavedSnapshot) -> Result<(), SandboxError> {
        self.records.put(record.id, record).await?;
        self.link(record.session, record.id).await
    }

    /// Writes the link from a session to one of its snapshots, unless it is already there.
    pub(crate) async fn ensure_link(
        &self,
        session: SessionId,
        id: SnapshotId,
    ) -> Result<(), SandboxError> {
        let path = self.links.join(session.to_string()).join(id.to_string());
        let present = tokio::fs::try_exists(&path)
            .await
            .map_err(|e| SandboxError::internal(format!("cannot read a snapshot link: {e}")))?;
        if present {
            return Ok(());
        }
        self.link(session, id).await
    }

    /// Writes the link from a session to one of its snapshots. Writing it again is harmless.
    pub(crate) async fn link(
        &self,
        session: SessionId,
        id: SnapshotId,
    ) -> Result<(), SandboxError> {
        let dir = self.links.join(session.to_string());
        create_private_dir(&dir)
            .map_err(|e| SandboxError::internal(format!("cannot create {}: {e}", dir.display())))?;
        write_file_atomic(dir.join(id.to_string()), Vec::new()).await
    }

    pub(crate) async fn get(&self, id: SnapshotId) -> Result<Option<SavedSnapshot>, SandboxError> {
        self.records.get(id).await
    }

    /// Every snapshot ID with a record.
    pub(crate) async fn ids(&self) -> Result<Vec<SnapshotId>, SandboxError> {
        self.records.ids().await
    }

    /// The IDs of the snapshots taken from one session, from its link directory.
    pub(crate) async fn of_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<SnapshotId>, SandboxError> {
        let dir = self.links.join(session.to_string());
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(SandboxError::internal(format!(
                    "cannot list snapshots: {e}"
                )))
            }
        };
        let mut ids = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| SandboxError::internal(format!("cannot list snapshots: {e}")))?
        {
            let name = entry.file_name();
            if let Some(id) = name.to_str().and_then(|n| n.parse::<SnapshotId>().ok()) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

/// The directory of parked sessions in a data directory.
pub fn parked_store(data_dir: &Path) -> Result<ParkedStore, SandboxError> {
    ParkedStore::open(&data_dir.join("sessions"))
}

/// The directory of snapshot records in a data directory.
pub(crate) fn snapshot_files(data_dir: &Path) -> Result<SnapshotFiles, SandboxError> {
    SnapshotFiles::open(data_dir)
}
