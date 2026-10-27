//! Copy-on-write virtual filesystem.
//!
//! Writes go to an in-memory overlay. `commit` persists the overlay to the
//! blob store as a Merkle tree and returns the new root hash. A `Vfs` built
//! from that root sees the committed tree.
//!
//! Commit is incremental. Only directories on a changed path are rewritten,
//! and every other subtree keeps its hash. Known limits are listed in
//! `README.md`.

use crate::error::{VfsError, VfsResult};
use crate::node::{Node, Stored};
use crate::ops::{DirEntry, FileHandle, FileStat, OpenFlags, SessionFs, Watch, WatchEvent};
use crate::quota::Quotas;
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::RwLock;
use sandbox_cas::{BlobStore, PutOptions};
use sandbox_types::hash::ContentHash;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Events a slow watcher can fall behind before it gets `Overflow`.
const EVENT_CAPACITY: usize = 1024;

#[cfg(test)]
mod tests;

/// Maximum symlink expansions while resolving one path (as in Linux, ELOOP).
const MAX_SYMLINK_HOPS: usize = 40;

pub struct Vfs {
    /// Root directory node of the last commit. `ContentHash::nil()` is an
    /// empty tree and is never read from the store.
    root: RwLock<ContentHash>,
    store: Arc<dyn BlobStore>,
    /// (bytes, inodes) of the committed tree, walked once and then kept
    /// current by `checkpoint`.
    committed_totals: RwLock<Option<(u64, u64)>>,
    overlay: RwLock<Overlay>,
    handles: RwLock<HashMap<u64, OpenFile>>,
    next_handle: AtomicU64,
    /// Change events for watchers, created by the first `watch` call. A VFS that
    /// nobody watches allocates nothing for events. See ADR 0005.
    events: std::sync::OnceLock<tokio::sync::broadcast::Sender<WatchEvent>>,
    quotas: Quotas,
    /// Absolute paths, in canonical form, exempt from quotas. The host writes them
    /// (for example `/etc/agent/env.json`), so they never use up a session's limits.
    exempt: Vec<String>,
}

#[derive(Default)]
struct Overlay {
    /// Paths created or changed since the last commit.
    nodes: BTreeMap<String, Node>,
    /// Committed paths hidden since the last commit. A path can be here and
    /// also in `nodes` after it is deleted and recreated. The overlay node
    /// wins, but the committed subtree beneath it stays hidden.
    whiteouts: BTreeSet<String>,
}

#[derive(Clone)]
struct OpenFile {
    path: String,
    flags: OpenFlags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Dir,
    File(u64),
    Symlink(String),
}

fn kind_of_node(node: &Node) -> Kind {
    match node {
        Node::Directory => Kind::Dir,
        Node::File(data) => Kind::File(data.len() as u64),
        Node::Symlink(target) => Kind::Symlink(target.clone()),
    }
}

fn kind_of_stored(stored: &Stored) -> Kind {
    match stored {
        Stored::Directory { .. } => Kind::Dir,
        Stored::File { size, .. } => Kind::File(*size),
        Stored::Symlink(target) => Kind::Symlink(target.clone()),
    }
}

impl Vfs {
    pub fn new(root: ContentHash, store: Arc<dyn BlobStore>) -> Self {
        Self {
            root: RwLock::new(root),
            store,
            committed_totals: RwLock::new(None),
            overlay: RwLock::new(Overlay::default()),
            handles: RwLock::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            events: std::sync::OnceLock::new(),
            quotas: Quotas::default(),
            exempt: Vec::new(),
        }
    }

    /// Sends an event to watchers. The event is built only when someone is
    /// watching, so an unwatched VFS pays one load per change.
    fn emit(&self, event: impl FnOnce() -> WatchEvent) {
        if let Some(tx) = self.events.get() {
            if tx.receiver_count() > 0 {
                // A send fails only when every watcher has gone away in the meantime.
                let _ = tx.send(event());
            }
        }
    }

    /// Handles that are open and not yet closed. Useful for leak checks.
    pub fn open_handle_count(&self) -> usize {
        self.handles.read().len()
    }

    pub fn with_quotas(mut self, quotas: Quotas) -> Self {
        self.quotas = quotas;
        self
    }

    /// Exempts files from quotas. They still take space in the tree, but writes to
    /// them are not checked and they are not counted in the usage that limits see.
    pub fn with_exempt_files(mut self, paths: &[&str]) -> Self {
        self.exempt = paths.iter().map(|path| path.to_string()).collect();
        self
    }

    fn is_exempt(&self, path: &str) -> bool {
        self.exempt.iter().any(|exempt| exempt == path)
    }

    /// Refuses an agent change to a host-owned file, or to a directory that holds
    /// one, so `rm -r /etc` cannot remove `env.json`. Host-owned files change only
    /// through `write_system_file`.
    fn guard_host_owned(&self, path: &str) -> VfsResult<()> {
        let guarded = self
            .exempt
            .iter()
            .any(|exempt| exempt == path || exempt.starts_with(&format!("{path}/")));
        if guarded {
            return Err(VfsError::PermissionDenied(format!(
                "{path} is managed by the host"
            )));
        }
        Ok(())
    }

    /// Writes a host-owned file. Its size is not checked against quotas, so the
    /// host must keep the file small. Agent writes to it are refused.
    pub async fn write_system_file(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        if !self.is_exempt(path) {
            return Err(VfsError::InvalidPath(format!(
                "{path} is not a host-owned file"
            )));
        }
        self.require_parent_dir(path).await?;
        if matches!(self.kind(path).await?, Some(Kind::Dir | Kind::Symlink(_))) {
            return Err(VfsError::InvalidPath(format!("not a regular file: {path}")));
        }
        self.put_file(path, data.to_vec());
        self.emit(|| WatchEvent::Modified(path.to_string()));
        Ok(())
    }

    /// Normalizes a path to `/a/b` form. `.` and empty segments are dropped.
    /// `..` is rejected rather than resolved, so no path can escape the root.
    fn normalize_path(&self, path: &Path) -> VfsResult<String> {
        let s = path
            .to_str()
            .ok_or_else(|| VfsError::InvalidPath("path is not valid UTF-8".to_string()))?;
        if s.contains('\0') {
            return Err(VfsError::InvalidPath("NUL byte in path".to_string()));
        }

        let mut parts = Vec::new();
        for part in s.split('/') {
            match part {
                "" | "." => continue,
                ".." => return Err(VfsError::InvalidPath(format!("'..' is not allowed: {s}"))),
                other => parts.push(other),
            }
        }

        if parts.len() > self.quotas.max_path_depth as usize {
            return Err(VfsError::QuotaExceeded(format!(
                "path depth exceeds {}",
                self.quotas.max_path_depth
            )));
        }

        if parts.is_empty() {
            Ok("/".to_string())
        } else {
            Ok(format!("/{}", parts.join("/")))
        }
    }

    /// Lexically normalizes `path`, then follows symlinks in its components.
    /// The final component is followed only when `follow_final` is set, so
    /// `readlink`, `rm`, and `rename` act on the link itself.
    async fn resolve_path(&self, path: &Path, follow_final: bool) -> VfsResult<String> {
        let mut current = self.normalize_path(path)?;
        let mut hops = 0usize;

        'restart: loop {
            let components: Vec<&str> = components(&current).collect();
            let mut resolved = String::from("/");

            for (i, name) in components.iter().enumerate() {
                let candidate = join(&resolved, name);
                let is_final = i + 1 == components.len();
                if is_final && !follow_final {
                    return Ok(candidate);
                }

                match self.kind(&candidate).await? {
                    Some(Kind::Symlink(target)) => {
                        hops += 1;
                        if hops > MAX_SYMLINK_HOPS {
                            return Err(VfsError::InvalidPath(format!(
                                "too many levels of symbolic links: {current}"
                            )));
                        }
                        // Relative targets resolve against the link's parent.
                        let base = if target.starts_with('/') {
                            target
                        } else {
                            format!("{resolved}/{target}")
                        };
                        let rest: String = components[i + 1..]
                            .iter()
                            .map(|c| format!("/{c}"))
                            .collect();
                        current = self.normalize_path(Path::new(&format!("{base}{rest}")))?;
                        continue 'restart;
                    }
                    // Missing or non-link: keep walking. A missing component
                    // makes the caller's own lookup fail with NotFound.
                    _ => resolved = candidate,
                }
            }

            return Ok(resolved);
        }
    }

    fn overlay_node(&self, path: &str) -> Option<Node> {
        self.overlay.read().nodes.get(path).cloned()
    }

    fn is_whiteout(&self, path: &str) -> bool {
        self.overlay.read().whiteouts.contains(path)
    }

    fn is_shadowed(&self, path: &str) -> bool {
        let overlay = self.overlay.read();
        overlay.nodes.contains_key(path) || overlay.whiteouts.contains(path)
    }

    async fn load_stored(&self, hash: &ContentHash) -> VfsResult<Stored> {
        let data = self
            .store
            .get(hash)
            .await?
            .ok_or_else(|| VfsError::NotFound(format!("node {hash}")))?;
        serde_json::from_slice(&data).map_err(|e| VfsError::Serialization(e.to_string()))
    }

    async fn put_node(&self, stored: &Stored) -> VfsResult<ContentHash> {
        let encoded =
            serde_json::to_vec(stored).map_err(|e| VfsError::Serialization(e.to_string()))?;
        Ok(self.store.put(&encoded, PutOptions::default()).await?)
    }

    /// The committed root directory node.
    async fn root_node(&self) -> VfsResult<Stored> {
        let root = *self.root.read();
        if root == ContentHash::nil() {
            return Ok(Stored::Directory {
                entries: BTreeMap::new(),
            });
        }
        self.load_stored(&root).await
    }

    /// The committed node at `path`, ignoring the overlay.
    async fn committed(&self, path: &str) -> VfsResult<Option<Stored>> {
        let mut current = self.root_node().await?;
        for name in components(path) {
            let next = match &current {
                Stored::Directory { entries } => match entries.get(name) {
                    Some(hash) => *hash,
                    None => return Ok(None),
                },
                _ => return Ok(None),
            };
            current = self.load_stored(&next).await?;
        }
        Ok(Some(current))
    }

    /// Child name to hash for the committed directory at `path`. Empty if the
    /// path is absent or not a committed directory.
    async fn committed_entries(&self, path: &str) -> VfsResult<BTreeMap<String, ContentHash>> {
        match self.committed(path).await? {
            Some(Stored::Directory { entries }) => Ok(entries),
            _ => Ok(BTreeMap::new()),
        }
    }

    /// Visible kind of `path`: overlay first, then whiteouts, then committed.
    async fn kind(&self, path: &str) -> VfsResult<Option<Kind>> {
        if path == "/" {
            return Ok(Some(Kind::Dir));
        }
        if let Some(node) = self.overlay_node(path) {
            return Ok(Some(kind_of_node(&node)));
        }
        if self.is_whiteout(path) {
            return Ok(None);
        }
        Ok(self.committed(path).await?.as_ref().map(kind_of_stored))
    }

    /// Full contents of a file, from the overlay or the blob store.
    /// `None` if the path is not a visible file.
    async fn file_bytes(&self, path: &str) -> VfsResult<Option<Vec<u8>>> {
        if let Some(node) = self.overlay_node(path) {
            return Ok(match node {
                Node::File(data) => Some(data),
                _ => None,
            });
        }
        if self.is_whiteout(path) {
            return Ok(None);
        }
        match self.committed(path).await? {
            Some(Stored::File { content, .. }) => {
                let data = self
                    .store
                    .get(&content)
                    .await?
                    .ok_or_else(|| VfsError::NotFound(format!("blob {content}")))?;
                Ok(Some(data.to_vec()))
            }
            _ => Ok(None),
        }
    }

    async fn require_parent_dir(&self, path: &str) -> VfsResult<()> {
        let parent = parent_of(path);
        match self.kind(parent).await? {
            Some(Kind::Dir) => Ok(()),
            Some(_) => Err(VfsError::InvalidPath(format!(
                "parent is not a directory: {parent}"
            ))),
            None => Err(VfsError::NotFound(parent.to_string())),
        }
    }

    /// Every visible path strictly under `path`, not including `path`.
    async fn descendants(&self, path: &str) -> VfsResult<BTreeSet<String>> {
        let prefix = child_prefix(path);
        let mut out = BTreeSet::new();
        {
            let overlay = self.overlay.read();
            for key in overlay.nodes.range(prefix.clone()..).map(|(k, _)| k) {
                if !key.starts_with(&prefix) {
                    break;
                }
                out.insert(key.clone());
            }
        }

        // Committed descendants. A whiteout hides its whole subtree, so the
        // walk does not descend into one.
        let mut stack = vec![path.to_string()];
        while let Some(dir) = stack.pop() {
            for name in self.committed_entries(&dir).await?.keys() {
                let child = join(&dir, name);
                if self.is_whiteout(&child) {
                    continue;
                }
                out.insert(child.clone());
                stack.push(child);
            }
        }
        Ok(out)
    }

    /// Committed (bytes, inodes), walked once. Later commits keep it current.
    async fn committed_totals(&self) -> VfsResult<(u64, u64)> {
        if let Some(totals) = *self.committed_totals.read() {
            return Ok(totals);
        }
        let totals = self.walk_totals().await?;
        *self.committed_totals.write() = Some(totals);
        Ok(totals)
    }

    async fn walk_totals(&self) -> VfsResult<(u64, u64)> {
        let mut bytes = 0u64;
        let mut inodes = 0u64;
        let mut stack = vec![self.root_node().await?];
        while let Some(node) = stack.pop() {
            if let Stored::Directory { entries } = node {
                for hash in entries.values() {
                    let child = self.load_stored(hash).await?;
                    inodes += 1;
                    if let Stored::File { size, .. } = &child {
                        bytes += size;
                    }
                    stack.push(child);
                }
            }
        }
        Ok((bytes, inodes))
    }

    /// Total bytes and inode count of the visible tree.
    ///
    /// Committed totals minus every committed path that is shadowed (in the
    /// overlay or whited out), plus the overlay's own nodes. Each shadowed
    /// path is one lookup, so the cost is proportional to the changes, not
    /// the tree.
    async fn usage(&self) -> VfsResult<(u64, u64)> {
        let (bytes, inodes) = self.raw_usage().await?;
        let (mut exempt_bytes, mut exempt_inodes) = (0u64, 0u64);
        for path in &self.exempt {
            if let Some(Kind::File(len)) = self.kind(path).await? {
                exempt_bytes += len;
                exempt_inodes += 1;
            }
        }
        Ok((
            bytes.saturating_sub(exempt_bytes),
            inodes.saturating_sub(exempt_inodes),
        ))
    }

    /// Total bytes and inodes of the tree, exempt files included. The committed
    /// totals cache holds this value.
    async fn raw_usage(&self) -> VfsResult<(u64, u64)> {
        let (mut bytes, mut inodes) = self.committed_totals().await?;

        let (shadowed, overlay_nodes): (BTreeSet<String>, Vec<(String, Node)>) = {
            let overlay = self.overlay.read();
            let shadowed = overlay
                .nodes
                .keys()
                .chain(overlay.whiteouts.iter())
                .cloned()
                .collect();
            let overlay_nodes = overlay
                .nodes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (shadowed, overlay_nodes)
        };

        for path in &shadowed {
            if let Some(stored) = self.committed(path).await? {
                inodes = inodes.saturating_sub(1);
                if let Stored::File { size, .. } = stored {
                    bytes = bytes.saturating_sub(size);
                }
            }
        }
        for (_, node) in &overlay_nodes {
            inodes += 1;
            if let Node::File(data) = node {
                bytes += data.len() as u64;
            }
        }
        Ok((bytes, inodes))
    }

    fn check_file_size(&self, size: u64) -> VfsResult<()> {
        if size > self.quotas.max_file_size {
            return Err(VfsError::QuotaExceeded(format!(
                "file size {size} exceeds max_file_size {}",
                self.quotas.max_file_size
            )));
        }
        Ok(())
    }

    /// Checks that one more inode fits within `inode_count`. Call before
    /// creating a file, directory, or symlink.
    async fn check_new_inode(&self) -> VfsResult<()> {
        let (_, inodes) = self.usage().await?;
        if inodes + 1 > self.quotas.inode_count {
            return Err(VfsError::QuotaExceeded(format!(
                "inode count {} would exceed {}",
                inodes + 1,
                self.quotas.inode_count
            )));
        }
        Ok(())
    }

    /// Checks that replacing a file of `old_len` bytes with `new_len` bytes
    /// keeps the tree within quota.
    async fn check_write_quota(&self, old_len: u64, new_len: u64) -> VfsResult<()> {
        self.check_file_size(new_len)?;
        let (bytes, _) = self.usage().await?;
        let after = bytes.saturating_sub(old_len).saturating_add(new_len);
        if after > self.quotas.total_bytes {
            return Err(VfsError::QuotaExceeded(format!(
                "total bytes {after} exceeds {}",
                self.quotas.total_bytes
            )));
        }
        Ok(())
    }

    fn handle(&self, handle: &FileHandle) -> VfsResult<OpenFile> {
        self.handles
            .read()
            .get(&handle.0)
            .cloned()
            .ok_or_else(|| VfsError::InvalidPath(format!("unknown file handle {}", handle.0)))
    }

    /// Stores file contents in the overlay, replacing whatever was there.
    fn put_file(&self, path: &str, data: Vec<u8>) {
        self.overlay
            .write()
            .nodes
            .insert(path.to_string(), Node::File(data));
    }
}

/// The non-empty segments of a normalized path.
fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|c| !c.is_empty())
}

/// `parent/name`, with the root handled.
fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}

fn child_prefix(path: &str) -> String {
    if path == "/" {
        "/".to_string()
    } else {
        format!("{path}/")
    }
}

/// Strict ancestors of a normalized path, from `/` down.
fn ancestors(path: &str) -> Vec<String> {
    let mut out = vec!["/".to_string()];
    let mut acc = String::new();
    let parts: Vec<&str> = components(path).collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        acc.push('/');
        acc.push_str(part);
        out.push(acc.clone());
    }
    if path == "/" {
        out.clear();
    }
    out
}

fn depth(path: &str) -> usize {
    components(path).count()
}

fn kind_stat(path: &str, kind: &Kind) -> FileStat {
    FileStat {
        path: path.to_string(),
        is_dir: *kind == Kind::Dir,
        is_file: matches!(kind, Kind::File(_)),
        is_symlink: matches!(kind, Kind::Symlink(_)),
        size: match kind {
            Kind::File(size) => *size,
            _ => 0,
        },
        mode: if *kind == Kind::Dir { 0o755 } else { 0o644 },
        modified: 0,
    }
}

#[async_trait]
impl SessionFs for Vfs {
    async fn total_bytes(&self) -> VfsResult<u64> {
        Ok(self.usage().await?.0)
    }

    async fn open(&self, path: &Path, flags: OpenFlags) -> VfsResult<FileHandle> {
        let path = self.resolve_path(path, true).await?;
        if !flags.read && !flags.write {
            return Err(VfsError::InvalidPath(
                "open needs read or write access".to_string(),
            ));
        }
        if flags.write || flags.create || flags.truncate {
            self.guard_host_owned(&path)?;
        }

        match self.kind(&path).await? {
            Some(Kind::Dir) => {
                return Err(VfsError::InvalidPath(format!("is a directory: {path}")))
            }
            // resolve_path follows symlinks, so a link can only be here if
            // resolution was skipped, which is a bug.
            Some(Kind::Symlink(_)) => {
                return Err(VfsError::Internal(format!("unresolved symlink at {path}")))
            }
            Some(Kind::File(_)) => {
                if flags.create && flags.exclusive {
                    return Err(VfsError::AlreadyExists(path));
                }
                if flags.truncate && flags.write {
                    let old_len = self.file_bytes(&path).await?.map_or(0, |d| d.len() as u64);
                    self.check_write_quota(old_len, 0).await?;
                    self.put_file(&path, Vec::new());
                    self.emit(|| WatchEvent::Modified(path.clone()));
                }
            }
            None => {
                if !flags.create {
                    return Err(VfsError::NotFound(path));
                }
                self.require_parent_dir(&path).await?;
                self.check_new_inode().await?;
                self.put_file(&path, Vec::new());
                self.emit(|| WatchEvent::Created(path.clone()));
            }
        }

        let id = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles.write().insert(id, OpenFile { path, flags });
        Ok(FileHandle(id))
    }

    async fn read(&self, handle: &FileHandle, offset: u64, len: usize) -> VfsResult<Bytes> {
        let open = self.handle(handle)?;
        if !open.flags.read {
            return Err(VfsError::PermissionDenied(format!(
                "handle not open for reading: {}",
                open.path
            )));
        }
        let data = self
            .file_bytes(&open.path)
            .await?
            .ok_or_else(|| VfsError::NotFound(open.path.clone()))?;

        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let end = start.saturating_add(len).min(data.len());
        Ok(Bytes::copy_from_slice(&data[start..end]))
    }

    async fn write(&self, handle: &FileHandle, offset: u64, data: &[u8]) -> VfsResult<usize> {
        let open = self.handle(handle)?;
        if !open.flags.write {
            return Err(VfsError::PermissionDenied(format!(
                "handle not open for writing: {}",
                open.path
            )));
        }
        let mut contents = self
            .file_bytes(&open.path)
            .await?
            .ok_or_else(|| VfsError::NotFound(open.path.clone()))?;

        let start = if open.flags.append {
            contents.len()
        } else {
            usize::try_from(offset)
                .map_err(|_| VfsError::InvalidPath("offset does not fit in memory".to_string()))?
        };
        let end = start
            .checked_add(data.len())
            .ok_or_else(|| VfsError::InvalidPath("write range overflows".to_string()))?;

        let old_len = contents.len() as u64;
        let new_len = old_len.max(end as u64);
        self.check_write_quota(old_len, new_len).await?;

        if contents.len() < end {
            // Writes past the end zero-fill the gap, like POSIX.
            contents.resize(end, 0);
        }
        contents[start..end].copy_from_slice(data);
        self.put_file(&open.path, contents);
        self.emit(|| WatchEvent::Modified(open.path.clone()));
        Ok(data.len())
    }

    async fn stat(&self, path: &Path) -> VfsResult<FileStat> {
        let path = self.resolve_path(path, true).await?;
        let kind = self
            .kind(&path)
            .await?
            .ok_or_else(|| VfsError::NotFound(path.clone()))?;
        Ok(kind_stat(&path, &kind))
    }

    async fn readdir(&self, path: &Path) -> VfsResult<Vec<DirEntry>> {
        let path = self.resolve_path(path, true).await?;
        match self.kind(&path).await? {
            Some(Kind::Dir) => {}
            Some(_) => return Err(VfsError::InvalidPath(format!("not a directory: {path}"))),
            None => return Err(VfsError::NotFound(path)),
        }

        let prefix = child_prefix(&path);
        let mut children: BTreeMap<String, Kind> = BTreeMap::new();

        // Overlay children. Writes require the parent to exist, so a path's
        // direct children are always explicit entries.
        {
            let overlay = self.overlay.read();
            for (key, node) in overlay.nodes.range(prefix.clone()..) {
                if !key.starts_with(&prefix) {
                    break;
                }
                if let Some(name) = direct_child(key, &prefix) {
                    children.insert(name.to_string(), kind_of_node(node));
                }
            }
        }

        // Committed children that the overlay has not shadowed.
        for (name, hash) in self.committed_entries(&path).await? {
            let child = join(&path, &name);
            if self.is_shadowed(&child) {
                continue;
            }
            let stored = self.load_stored(&hash).await?;
            children
                .entry(name)
                .or_insert_with(|| kind_of_stored(&stored));
        }

        Ok(children
            .into_iter()
            .map(|(name, kind)| {
                let stat = kind_stat(&name, &kind);
                DirEntry {
                    name,
                    is_dir: stat.is_dir,
                    is_file: stat.is_file,
                    is_symlink: stat.is_symlink,
                    size: stat.size,
                    mode: stat.mode,
                }
            })
            .collect())
    }

    async fn mkdir(&self, path: &Path, _mode: u32) -> VfsResult<()> {
        let path = self.resolve_path(path, false).await?;
        if path == "/" || self.kind(&path).await?.is_some() {
            return Err(VfsError::AlreadyExists(path));
        }
        self.require_parent_dir(&path).await?;
        self.check_new_inode().await?;

        self.overlay
            .write()
            .nodes
            .insert(path.clone(), Node::Directory);
        self.emit(|| WatchEvent::Created(path));
        Ok(())
    }

    async fn rm(&self, path: &Path, recursive: bool) -> VfsResult<()> {
        let path = self.resolve_path(path, false).await?;
        if path == "/" {
            return Err(VfsError::InvalidPath("cannot remove the root".to_string()));
        }
        self.guard_host_owned(&path)?;
        let kind = self
            .kind(&path)
            .await?
            .ok_or_else(|| VfsError::NotFound(path.clone()))?;

        // A file has no descendants, so this is empty unless `path` is a directory.
        let mut doomed = if kind == Kind::Dir {
            self.descendants(&path).await?
        } else {
            BTreeSet::new()
        };
        if !doomed.is_empty() && !recursive {
            return Err(VfsError::InvalidPath(format!(
                "directory not empty: {path}"
            )));
        }
        doomed.insert(path);

        let mut overlay = self.overlay.write();
        for key in doomed {
            overlay.nodes.remove(&key);
            overlay.whiteouts.insert(key.clone());
            self.emit(|| WatchEvent::Removed(key));
        }
        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> VfsResult<()> {
        let from = self.resolve_path(from, false).await?;
        let to = self.resolve_path(to, false).await?;
        if from == to {
            return Ok(());
        }
        if from == "/" || to == "/" {
            return Err(VfsError::InvalidPath("cannot rename the root".to_string()));
        }
        self.guard_host_owned(&from)?;
        self.guard_host_owned(&to)?;
        if to.starts_with(&child_prefix(&from)) {
            return Err(VfsError::InvalidPath(format!(
                "cannot move {from} into itself"
            )));
        }
        if self.kind(&from).await?.is_none() {
            return Err(VfsError::NotFound(from));
        }
        if self.kind(&to).await?.is_some() {
            return Err(VfsError::AlreadyExists(to));
        }
        self.require_parent_dir(&to).await?;

        // Build every moved node before changing anything, so a failure
        // partway through cannot leave the tree half-moved.
        let mut sources = self.descendants(&from).await?;
        sources.insert(from.clone());
        let mut moved = Vec::with_capacity(sources.len());
        for old in &sources {
            let new = format!("{to}{}", &old[from.len()..]);
            let kind = self
                .kind(old)
                .await?
                .ok_or_else(|| VfsError::NotFound(old.clone()))?;
            let node = match kind {
                Kind::Dir => Node::Directory,
                Kind::File(_) => Node::File(
                    self.file_bytes(old)
                        .await?
                        .ok_or_else(|| VfsError::NotFound(old.clone()))?,
                ),
                Kind::Symlink(target) => Node::Symlink(target),
            };
            moved.push((old.clone(), new, node));
        }

        let mut overlay = self.overlay.write();
        for (old, _, _) in &moved {
            overlay.nodes.remove(old);
            overlay.whiteouts.insert(old.clone());
        }
        for (old, new, node) in moved {
            overlay.nodes.insert(new.clone(), node);
            self.emit(|| WatchEvent::Renamed { from: old, to: new });
        }
        Ok(())
    }

    async fn symlink(&self, target: &Path, link: &Path) -> VfsResult<()> {
        let link = self.resolve_path(link, false).await?;
        let target = target
            .to_str()
            .ok_or_else(|| VfsError::InvalidPath("symlink target is not valid UTF-8".to_string()))?
            .to_string();
        self.guard_host_owned(&link)?;
        if self.kind(&link).await?.is_some() {
            return Err(VfsError::AlreadyExists(link));
        }
        self.require_parent_dir(&link).await?;
        self.check_new_inode().await?;

        self.overlay
            .write()
            .nodes
            .insert(link.clone(), Node::Symlink(target));
        self.emit(|| WatchEvent::Created(link));
        Ok(())
    }

    async fn readlink(&self, path: &Path) -> VfsResult<String> {
        let path = self.resolve_path(path, false).await?;
        match self.kind(&path).await? {
            Some(Kind::Symlink(target)) => Ok(target),
            Some(_) => Err(VfsError::InvalidPath(format!("not a symlink: {path}"))),
            None => Err(VfsError::NotFound(path)),
        }
    }

    async fn truncate(&self, path: &Path, size: u64) -> VfsResult<()> {
        let path = self.resolve_path(path, true).await?;
        self.guard_host_owned(&path)?;
        let old = match self.kind(&path).await? {
            Some(Kind::File(len)) => len,
            Some(_) => return Err(VfsError::InvalidPath(format!("not a regular file: {path}"))),
            None => return Err(VfsError::NotFound(path)),
        };
        self.check_write_quota(old, size).await?;

        let mut contents = self
            .file_bytes(&path)
            .await?
            .ok_or_else(|| VfsError::NotFound(path.clone()))?;
        let size = usize::try_from(size)
            .map_err(|_| VfsError::InvalidPath("size does not fit in memory".to_string()))?;
        contents.resize(size, 0);
        self.put_file(&path, contents);
        self.emit(|| WatchEvent::Modified(path));
        Ok(())
    }

    fn close(&self, handle: &FileHandle) {
        self.handles.write().remove(&handle.0);
    }

    async fn watch(&self, path: &Path) -> VfsResult<Watch> {
        let prefix = self.normalize_path(path)?;
        let tx = self
            .events
            .get_or_init(|| tokio::sync::broadcast::channel(EVENT_CAPACITY).0);
        Ok(Watch::new(prefix, tx.subscribe()))
    }

    /// Persists the overlay and returns the new root.
    ///
    /// Only directories on a changed path are rewritten, bottom-up, so the
    /// cost scales with the changes, not the tree. Every other subtree keeps
    /// its hash. Visible content is unchanged by a commit, so the cached
    /// totals stay correct.
    async fn commit(&mut self) -> VfsResult<ContentHash> {
        self.checkpoint().await
    }
}

/// The name of `key` if it is a direct child of the directory `prefix`.
fn direct_child<'a>(key: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = key.strip_prefix(prefix)?;
    if rest.is_empty() || rest.contains('/') {
        None
    } else {
        Some(rest)
    }
}

impl Vfs {
    /// Writes the visible tree to the store and returns its root hash. The
    /// session keeps working afterwards. Writes made while this runs may not be
    /// in the returned root, so take snapshots while the session is idle.
    pub async fn checkpoint(&self) -> VfsResult<ContentHash> {
        let visible = self.raw_usage().await?;
        let (overlay_nodes, whiteouts) = {
            let overlay = self.overlay.read();
            (overlay.nodes.clone(), overlay.whiteouts.clone())
        };

        // Directories whose entries change: ancestors of every changed path,
        // plus every overlay directory.
        let mut dirty: BTreeSet<String> = BTreeSet::from(["/".to_string()]);
        let mut changed_children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for path in overlay_nodes.keys().chain(whiteouts.iter()) {
            for ancestor in ancestors(path) {
                dirty.insert(ancestor);
            }
        }
        for (path, node) in &overlay_nodes {
            if matches!(node, Node::Directory) {
                dirty.insert(path.clone());
            }
        }
        for path in overlay_nodes
            .keys()
            .chain(whiteouts.iter())
            .chain(dirty.iter())
        {
            if path != "/" {
                let name = components(path).last().unwrap_or_default().to_string();
                changed_children
                    .entry(parent_of(path).to_string())
                    .or_default()
                    .insert(name);
            }
        }

        // Leaves first: file contents and symlink nodes from the overlay.
        let mut leaf: HashMap<String, ContentHash> = HashMap::new();
        for (path, node) in &overlay_nodes {
            let stored = match node {
                Node::File(data) => {
                    let content = self.store.put(data, PutOptions::default()).await?;
                    Stored::File {
                        content,
                        size: data.len() as u64,
                    }
                }
                Node::Symlink(target) => Stored::Symlink(target.clone()),
                Node::Directory => continue,
            };
            leaf.insert(path.clone(), self.put_node(&stored).await?);
        }

        // Directories deepest first, so each child's hash is known before its parent.
        let mut order: Vec<String> = dirty.iter().cloned().collect();
        order.sort_by_key(|p| std::cmp::Reverse(depth(p)));
        let mut dir_hash: HashMap<String, Option<ContentHash>> = HashMap::new();

        for dir in &order {
            let replaced_or_removed = whiteouts.contains(dir);
            let overlay_dir = matches!(overlay_nodes.get(dir), Some(Node::Directory));
            if replaced_or_removed && !overlay_dir {
                dir_hash.insert(dir.clone(), None);
                continue;
            }

            // A deleted-then-recreated directory hides its committed children.
            let mut entries = if replaced_or_removed {
                BTreeMap::new()
            } else {
                self.committed_entries(dir).await?
            };

            for name in changed_children.get(dir).into_iter().flatten() {
                let child = join(dir, name);
                let new_hash = if let Some(Node::Directory) = overlay_nodes.get(&child) {
                    dir_hash.get(&child).copied().flatten()
                } else if overlay_nodes.contains_key(&child) {
                    leaf.get(&child).copied()
                } else if dirty.contains(&child) {
                    dir_hash.get(&child).copied().flatten()
                } else if whiteouts.contains(&child) {
                    None
                } else {
                    // Untouched committed child: keep its entry.
                    continue;
                };
                match new_hash {
                    Some(hash) => entries.insert(name.clone(), hash),
                    None => entries.remove(name),
                };
            }

            let hash = self.put_node(&Stored::Directory { entries }).await?;
            dir_hash.insert(dir.clone(), Some(hash));
        }

        let root = dir_hash
            .get("/")
            .copied()
            .flatten()
            .ok_or_else(|| VfsError::Internal("root directory was not rebuilt".to_string()))?;

        *self.root.write() = root;
        *self.committed_totals.write() = Some(visible);
        let mut overlay = self.overlay.write();
        overlay.nodes.clear();
        overlay.whiteouts.clear();
        Ok(root)
    }
}
