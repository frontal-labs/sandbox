use crate::blob_store::{BlobStore, BlobStoreResult, PutOptions};
use sandbox_types::hash::ContentHash;
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct MemStore {
    blobs: Arc<RwLock<HashMap<ContentHash, Bytes>>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            blobs: Arc::new(RwLock::new(HashMap::with_capacity(capacity))),
        }
    }

    pub fn len(&self) -> usize {
        self.blobs.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.read().is_empty()
    }

    pub fn clear(&self) {
        self.blobs.write().clear();
    }

    fn put_raw(&self, hash: ContentHash, data: Bytes) {
        self.blobs.write().insert(hash, data);
    }
}

#[async_trait]
impl BlobStore for MemStore {
    async fn get(&self, hash: &ContentHash) -> BlobStoreResult<Option<Bytes>> {
        Ok(self.blobs.read().get(hash).cloned())
    }

    async fn put(&self, data: &[u8], _options: PutOptions) -> BlobStoreResult<ContentHash> {
        let hash = ContentHash::new(data);
        let bytes = Bytes::copy_from_slice(data);
        self.put_raw(hash, bytes);
        Ok(hash)
    }

    async fn has(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        Ok(self.blobs.read().contains_key(hash))
    }

    async fn delete(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        Ok(self.blobs.write().remove(hash).is_some())
    }

    async fn list(&self, _prefix: Option<&str>) -> BlobStoreResult<Vec<ContentHash>> {
        Ok(self.blobs.read().keys().copied().collect())
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_store::PutOptions;

    #[tokio::test]
    async fn test_mem_store_basic() {
        let store = MemStore::new();
        let data = b"hello world";

        let hash = store.put(data, PutOptions::default()).await.unwrap();
        assert!(store.has(&hash).await.unwrap());

        let retrieved = store.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved.as_ref(), data);

        let deleted = store.delete(&hash).await.unwrap();
        assert!(deleted);
        assert!(!store.has(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_mem_store_duplicate() {
        let store = MemStore::new();
        let data = b"duplicate test";

        let hash1 = store.put(data, PutOptions::default()).await.unwrap();
        let hash2 = store.put(data, PutOptions::default()).await.unwrap();

        assert_eq!(hash1, hash2);
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn test_mem_store_not_found() {
        let store = MemStore::new();
        let hash = ContentHash::nil();

        let result = store.get(&hash).await.unwrap();
        assert!(result.is_none());

        let has = store.has(&hash).await.unwrap();
        assert!(!has);
    }
}