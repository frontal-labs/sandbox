use crate::blob_store::{BlobStore, BlobStoreError, BlobStoreResult, PutOptions};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::StreamExt;
use object_store::{path::Path as ObjectPath, ObjectStore, ObjectStoreExt};
use sandbox_types::hash::ContentHash;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, trace};

#[derive(Debug)]
pub struct TieredStore {
    local: Arc<dyn BlobStore>,
    remote: Arc<dyn ObjectStore>,
    cache_policy: CachePolicy,
    stats: Arc<RwLock<TieredStats>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    WriteThrough,
    WriteBack,
    ReadThrough,
}

#[derive(Debug, Clone, Default)]
pub struct TieredStats {
    local_hits: u64,
    remote_hits: u64,
    misses: u64,
    writes: u64,
}

impl TieredStore {
    pub fn new(
        local: Arc<dyn BlobStore>,
        remote: Arc<dyn ObjectStore>,
        cache_policy: CachePolicy,
    ) -> Self {
        Self {
            local,
            remote,
            cache_policy,
            stats: Arc::new(RwLock::new(TieredStats::default())),
        }
    }

    pub fn local_store(&self) -> &Arc<dyn BlobStore> {
        &self.local
    }

    pub fn remote_store(&self) -> &Arc<dyn ObjectStore> {
        &self.remote
    }

    pub async fn stats(&self) -> TieredStats {
        self.stats.read().await.clone()
    }

    pub fn object_path(hash: &ContentHash) -> ObjectPath {
        ObjectPath::from(hash.to_hex())
    }
}

#[async_trait]
impl BlobStore for TieredStore {
    async fn get(&self, hash: &ContentHash) -> BlobStoreResult<Option<Bytes>> {
        trace!("TieredStore get: {}", hash);

        if let Some(data) = self.local.get(hash).await? {
            self.stats.write().await.local_hits += 1;
            return Ok(Some(data));
        }

        let path = Self::object_path(hash);
        match self.remote.get(&path).await {
            Ok(result) => {
                let data = result.bytes().await.map_err(|e| {
                    BlobStoreError::Storage(format!("Failed to read remote object: {}", e))
                })?;

                // The bucket is not trusted. Bytes that do not hash to the requested
                // value are never returned or cached, as in `DiskStore`.
                if ContentHash::new(&data) != *hash {
                    return Err(BlobStoreError::Storage(format!(
                        "remote blob {hash} does not match its hash; the remote store is corrupt or tampered with"
                    )));
                }

                if matches!(
                    self.cache_policy,
                    CachePolicy::ReadThrough | CachePolicy::WriteThrough
                ) {
                    self.local.put(&data, PutOptions::default()).await?;
                }

                self.stats.write().await.remote_hits += 1;
                Ok(Some(data))
            }
            Err(object_store::Error::NotFound { .. }) => {
                self.stats.write().await.misses += 1;
                Ok(None)
            }
            Err(e) => Err(BlobStoreError::Storage(format!(
                "Remote store error: {}",
                e
            ))),
        }
    }

    async fn put(&self, data: &[u8], options: PutOptions) -> BlobStoreResult<ContentHash> {
        let hash = ContentHash::new(data);

        // A blob the local store already has may still be missing from the remote, if it was
        // written before the remote was configured or its remote write failed. So the remote
        // is checked on every put, not only the first.
        if !self.local.has(&hash).await? {
            self.local.put(data, options).await?;
        }

        if matches!(
            self.cache_policy,
            CachePolicy::WriteThrough | CachePolicy::WriteBack
        ) {
            let path = Self::object_path(&hash);
            match self.remote.head(&path).await {
                Ok(_) => {}
                Err(object_store::Error::NotFound { .. }) => {
                    self.remote
                        .put(&path, Bytes::copy_from_slice(data).into())
                        .await
                        .map_err(|e| {
                            BlobStoreError::Storage(format!("Failed to write to remote: {}", e))
                        })?;
                }
                Err(e) => {
                    return Err(BlobStoreError::Storage(format!(
                        "Remote store error: {}",
                        e
                    )))
                }
            }
        }

        self.stats.write().await.writes += 1;
        debug!("TieredStore put: {} ({} bytes)", hash, data.len());
        Ok(hash)
    }

    async fn has(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        if self.local.has(hash).await? {
            return Ok(true);
        }

        let path = Self::object_path(hash);
        match self.remote.head(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(BlobStoreError::Storage(format!(
                "Remote store error: {}",
                e
            ))),
        }
    }

    async fn delete(&self, hash: &ContentHash) -> BlobStoreResult<bool> {
        let local_deleted = self.local.delete(hash).await?;

        let path = Self::object_path(hash);
        match self.remote.delete(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(local_deleted),
            Err(e) => Err(BlobStoreError::Storage(format!(
                "Remote store error: {}",
                e
            ))),
        }
    }

    async fn list_parts(&self) -> BlobStoreResult<Vec<String>> {
        self.local.list_parts().await
    }

    async fn parts(&self, hash: &ContentHash) -> BlobStoreResult<Vec<ContentHash>> {
        self.local.parts(hash).await
    }

    async fn delete_parts(
        &self,
        candidates: &[String],
        keep: &std::collections::HashSet<ContentHash>,
    ) -> BlobStoreResult<usize> {
        self.local.delete_parts(candidates, keep).await
    }

    async fn list(&self, prefix: Option<&str>) -> BlobStoreResult<Vec<ContentHash>> {
        let mut results = Vec::new();
        let prefix_path = prefix.map(ObjectPath::from);

        let mut stream = self.remote.list(prefix_path.as_ref());
        while let Some(result) = stream.next().await {
            let meta =
                result.map_err(|e| BlobStoreError::Storage(format!("Remote list error: {}", e)))?;
            if let Some(filename) = meta.location.filename() {
                if let Ok(hash) = ContentHash::from_hex(filename) {
                    results.push(hash);
                }
            }
        }

        Ok(results)
    }

    fn name(&self) -> &'static str {
        "tiered"
    }
}
