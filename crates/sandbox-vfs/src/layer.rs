use sandbox_types::hash::ContentHash;
use sandbox_cas::BlobStore;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Layer {
    pub root: ContentHash,
    pub nodes: HashMap<String, ContentHash>,
    pub store: Arc<dyn BlobStore>,
}

impl Layer {
    pub fn new(root: ContentHash, store: Arc<dyn BlobStore>) -> Self {
        Self {
            root,
            nodes: HashMap::new(),
            store,
        }
    }

    pub fn get(&self, path: &str) -> Option<&ContentHash> {
        self.nodes.get(path)
    }

    pub fn insert(&mut self, path: String, hash: ContentHash) {
        self.nodes.insert(path, hash);
    }

    pub fn remove(&mut self, path: &str) -> Option<ContentHash> {
        self.nodes.remove(path)
    }
}