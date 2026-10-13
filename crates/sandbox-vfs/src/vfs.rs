use crate::error::{VfsError, VfsResult};
use crate::layer::Layer;
use crate::node::{Directory, FileNode, Node};
use crate::ops::{DirEntry, FileHandle, FileStat, OpenFlags, SessionFs, WatchHandle};
use crate::quota::{Quotas, Usage};
use sandbox_cas::BlobStore;
use sandbox_types::hash::ContentHash;
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::RwLock;
use serde_json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Vfs {
    base: Layer,
    overlay: RwLock<Overlay>,
    quotas: Quotas,
    _usage: RwLock<Usage>,
    store: Arc<dyn BlobStore>,
    next_handle: std::sync::atomic::AtomicU64,
    next_watch: std::sync::atomic::AtomicU64,
    watchers: RwLock<HashMap<u64, mpsc::Sender<WatchEvent>>>,
}

struct Overlay {
    nodes: HashMap<String, Node>,
    whiteouts: HashMap<String, ()>,
    dirty_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed(PathBuf, PathBuf),
}

impl Vfs {
    pub fn new(root: ContentHash, store: Arc<dyn BlobStore>) -> Self {
        let base = Layer::new(root, store.clone());
        Self {
            base,
            overlay: RwLock::new(Overlay {
                nodes: HashMap::new(),
                whiteouts: HashMap::new(),
                dirty_paths: Vec::new(),
            }),
            quotas: Quotas::default(),
            _usage: RwLock::new(Usage::default()),
            store,
            next_handle: std::sync::atomic::AtomicU64::new(1),
            next_watch: std::sync::atomic::AtomicU64::new(1),
            watchers: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_quotas(mut self, quotas: Quotas) -> Self {
        self.quotas = quotas;
        self
    }

    fn normalize_path(&self, path: &Path) -> VfsResult<String> {
        let path_str = path.to_string_lossy().to_string();
        if path_str.contains('\0') {
            return Err(VfsError::InvalidPath("NUL byte in path".to_string()));
        }
        let normalized = path_str.trim_start_matches('/');
        if normalized.is_empty() {
            Ok("/".to_string())
        } else {
            Ok(format!("/{}", normalized))
        }
    }

    async fn resolve(&self, path: &str) -> Option<Node> {
        let hash = {
            let overlay = self.overlay.read();

            if overlay.whiteouts.contains_key(path) {
                return None;
            }

            if let Some(node) = overlay.nodes.get(path) {
                return Some(node.clone());
            }

            self.base.get(path).cloned()
        };

        if let Some(hash) = hash {
            return self.load_node(&hash).await;
        }

        None
    }

    async fn load_node(&self, hash: &ContentHash) -> Option<Node> {
        let data = self.store.get(hash).await.ok()??;
        serde_json::from_slice(&data).ok()
    }

    fn mark_dirty(&self, path: &str) {
        self.overlay.write().dirty_paths.push(path.to_string());
    }

    fn allocate_handle(&self) -> FileHandle {
        FileHandle(self.next_handle.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    fn allocate_watch(&self) -> WatchHandle {
        WatchHandle(self.next_watch.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    fn emit_watch(&self, event: WatchEvent) {
        let watchers = self.watchers.read();
        for tx in watchers.values() {
            let _ = tx.try_send(event.clone());
        }
    }
}

#[async_trait]
impl SessionFs for Vfs {
    async fn open(&self, path: &Path, flags: OpenFlags) -> VfsResult<FileHandle> {
        let path = self.normalize_path(path)?;

        if flags.create && flags.exclusive && self.resolve(&path).await.is_some() {
            return Err(VfsError::AlreadyExists(path));
        }

        if flags.create && self.resolve(&path).await.is_none() {
            let node = Node::File(FileNode::new(Vec::new(), 0));
            self.overlay.write().nodes.insert(path.clone(), node);
            self.mark_dirty(&path);
            self.emit_watch(WatchEvent::Created(PathBuf::from(&path)));
        }

        let handle = self.allocate_handle();
        Ok(handle)
    }

    async fn read(&self, _handle: &FileHandle, _offset: u64, _len: usize) -> VfsResult<Bytes> {
        Ok(Bytes::new())
    }

    async fn write(&self, _handle: &FileHandle, _offset: u64, _data: &[u8]) -> VfsResult<usize> {
        Ok(0)
    }

    async fn stat(&self, path: &Path) -> VfsResult<FileStat> {
        let path = self.normalize_path(path)?;
        let node = self.resolve(&path).await.ok_or(VfsError::NotFound(path.clone()))?;

        Ok(FileStat {
            path: path.clone(),
            is_dir: node.is_dir(),
            is_file: node.is_file(),
            is_symlink: node.is_symlink(),
            size: match &node {
                Node::File(f) => f.size,
                _ => 0,
            },
            mode: 0o644,
            modified: 0,
        })
    }

    async fn readdir(&self, path: &Path) -> VfsResult<Vec<DirEntry>> {
        let path = self.normalize_path(path)?;
        let node = self.resolve(&path).await.ok_or(VfsError::NotFound(path))?;

        let dir = match node {
            Node::Directory(dir) => dir,
            _ => return Err(VfsError::InvalidPath("Not a directory".to_string())),
        };

        let mut entries = Vec::new();
        for entry in dir.iter() {
            entries.push(DirEntry {
                name: entry.name.clone(),
                is_dir: false,
                is_file: true,
                is_symlink: false,
                size: 0,
                mode: entry.mode,
            });
        }
        Ok(entries)
    }

    async fn mkdir(&self, path: &Path, _mode: u32) -> VfsResult<()> {
        let path = self.normalize_path(path)?;
        if self.resolve(&path).await.is_some() {
            return Err(VfsError::AlreadyExists(path));
        }

        let node = Node::Directory(Directory::new());
        self.overlay.write().nodes.insert(path.clone(), node);
        self.mark_dirty(&path);
        self.emit_watch(WatchEvent::Created(PathBuf::from(&path)));
        Ok(())
    }

    async fn rm(&self, path: &Path, _recursive: bool) -> VfsResult<()> {
        let path = self.normalize_path(path)?;
        self.overlay.write().whiteouts.insert(path.clone(), ());
        self.overlay.write().nodes.remove(&path);
        self.mark_dirty(&path);
        self.emit_watch(WatchEvent::Deleted(PathBuf::from(&path)));
        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> VfsResult<()> {
        let from = self.normalize_path(from)?;
        let to = self.normalize_path(to)?;

        let node = self.resolve(&from).await.ok_or(VfsError::NotFound(from.clone()))?;
        self.overlay.write().nodes.remove(&from);
        self.overlay.write().whiteouts.insert(from.clone(), ());
        self.overlay.write().nodes.insert(to.clone(), node);
        self.mark_dirty(&from);
        self.mark_dirty(&to);
        self.emit_watch(WatchEvent::Renamed(PathBuf::from(&from), PathBuf::from(&to)));
        Ok(())
    }

    async fn symlink(&self, target: &Path, link: &Path) -> VfsResult<()> {
        let link = self.normalize_path(link)?;
        let target = target.to_string_lossy().to_string();

        if self.resolve(&link).await.is_some() {
            return Err(VfsError::AlreadyExists(link));
        }

        let node = Node::Symlink(target);
        self.overlay.write().nodes.insert(link.clone(), node);
        self.mark_dirty(&link);
        self.emit_watch(WatchEvent::Created(PathBuf::from(&link)));
        Ok(())
    }

    async fn readlink(&self, path: &Path) -> VfsResult<String> {
        let path = self.normalize_path(path)?;
        let node = self.resolve(&path).await.ok_or(VfsError::NotFound(path))?;

        match node {
            Node::Symlink(target) => Ok(target),
            _ => Err(VfsError::InvalidPath("Not a symlink".to_string())),
        }
    }

    async fn truncate(&self, path: &Path, size: u64) -> VfsResult<()> {
        let path = self.normalize_path(path)?;
        let mut overlay = self.overlay.write();

        if let Some(Node::File(file)) = overlay.nodes.get_mut(&path) {
            file.size = size;
            self.mark_dirty(&path);
            Ok(())
        } else if self.base.get(&path).is_some() {
            let file = FileNode::new(Vec::new(), size);
            overlay.nodes.insert(path.clone(), Node::File(file));
            self.mark_dirty(&path);
            Ok(())
        } else {
            Err(VfsError::NotFound(path))
        }
    }

    async fn watch(&self, _path: &Path) -> VfsResult<WatchHandle> {
        let handle = self.allocate_watch();
        let (tx, _) = mpsc::channel(100);
        self.watchers.write().insert(handle.0, tx);
        Ok(handle)
    }

    async fn commit(&mut self) -> VfsResult<ContentHash> {
        let dirty_paths = {
            let mut overlay = self.overlay.write();
            std::mem::take(&mut overlay.dirty_paths)
        };

        for path in dirty_paths {
            let node = {
                let overlay = self.overlay.read();
                overlay.nodes.get(&path).cloned()
            };
            if let Some(node) = node {
                let data = serde_json::to_vec(&node).map_err(|e| {
                    VfsError::Serialization(e.to_string())
                })?;
                let hash = self.store.put(&data, Default::default()).await?;
                self.base.insert(path.clone(), hash);
            }
        }

        let root_data = serde_json::to_vec(&self.base.nodes).map_err(|e| {
            VfsError::Serialization(e.to_string())
        })?;
        let root_hash = self.store.put(&root_data, Default::default()).await?;
        Ok(root_hash)
    }
}