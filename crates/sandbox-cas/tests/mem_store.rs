use sandbox_cas::blob_store::PutOptions;
use sandbox_cas::mem_store::MemStore;
use sandbox_cas::BlobStore;
use sandbox_types::hash::ContentHash;

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
