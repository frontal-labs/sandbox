use bytes::Bytes;
use object_store::memory::InMemory;
use object_store::ObjectStoreExt;
use sandbox_cas::blob_store::PutOptions;
use sandbox_cas::mem_store::MemStore;
use sandbox_cas::tiered_store::{CachePolicy, TieredStore};
use sandbox_cas::BlobStore;
use sandbox_types::hash::ContentHash;
use std::sync::Arc;

#[tokio::test]
async fn test_tiered_store_read_through() {
    let local = Arc::new(MemStore::new());
    let remote = Arc::new(InMemory::new());
    let store = TieredStore::new(local.clone(), remote.clone(), CachePolicy::ReadThrough);

    let data = b"tiered test";
    let hash = ContentHash::new(data);

    remote
        .put(
            &TieredStore::object_path(&hash),
            Bytes::from_static(data).into(),
        )
        .await
        .unwrap();

    let retrieved = store.get(&hash).await.unwrap().unwrap();
    assert_eq!(retrieved.as_ref(), data);

    assert!(local.has(&hash).await.unwrap());
}

#[tokio::test]
async fn a_remote_blob_with_the_wrong_content_is_rejected_and_not_cached() {
    let local = Arc::new(MemStore::new());
    let remote = Arc::new(InMemory::new());
    let store = TieredStore::new(local.clone(), remote.clone(), CachePolicy::ReadThrough);

    // The remote serves other bytes under this hash.
    let wanted = ContentHash::new(b"the content that was asked for");
    remote
        .put(
            &TieredStore::object_path(&wanted),
            Bytes::from_static(b"tampered content").into(),
        )
        .await
        .unwrap();

    let err = store.get(&wanted).await.unwrap_err();
    assert!(
        err.to_string().contains("does not match its hash"),
        "unexpected error: {err}"
    );
    assert!(!local.has(&wanted).await.unwrap(), "bad bytes were cached");
}

#[tokio::test]
async fn a_blob_the_local_store_has_still_reaches_the_remote() {
    let local = Arc::new(MemStore::new());
    let remote = Arc::new(InMemory::new());
    let store = TieredStore::new(local.clone(), remote.clone(), CachePolicy::WriteThrough);

    // Written locally before the remote existed, so only the local store has it.
    let data = b"written before the remote";
    let hash = local.put(data, PutOptions::default()).await.unwrap();
    assert!(remote.head(&TieredStore::object_path(&hash)).await.is_err());

    store.put(data, PutOptions::default()).await.unwrap();
    assert!(remote.head(&TieredStore::object_path(&hash)).await.is_ok());
}

#[tokio::test]
async fn test_tiered_store_write_through() {
    let local = Arc::new(MemStore::new());
    let remote = Arc::new(InMemory::new());
    let store = TieredStore::new(local.clone(), remote.clone(), CachePolicy::WriteThrough);

    let data = b"write through test";
    let hash = store.put(data, PutOptions::default()).await.unwrap();

    assert!(local.has(&hash).await.unwrap());
    assert!(remote.head(&TieredStore::object_path(&hash)).await.is_ok());
}
