use sandbox_types::hash::ContentHash;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub mode: u32,
    pub hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    pub entries: BTreeMap<String, DirEntry>,
}

impl Directory {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, mode: u32, hash: ContentHash) {
        self.entries.insert(name.clone(), DirEntry { name, mode, hash });
    }

    pub fn remove(&mut self, name: &str) -> Option<DirEntry> {
        self.entries.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&DirEntry> {
        self.entries.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DirEntry> {
        self.entries.values()
    }
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNode {
    pub chunks: Vec<ContentHash>,
    pub size: u64,
}

impl FileNode {
    pub fn new(chunks: Vec<ContentHash>, size: u64) -> Self {
        Self { chunks, size }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Node {
    Directory(Directory),
    File(FileNode),
    Symlink(String),
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self, Node::Directory(_))
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Node::File(_))
    }

    pub fn is_symlink(&self) -> bool {
        matches!(self, Node::Symlink(_))
    }
}