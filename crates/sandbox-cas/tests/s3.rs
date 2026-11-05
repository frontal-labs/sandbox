//! A blob written on one node is read on another, through an S3-compatible object store. The
//! two nodes have separate local directories, so the read can only come from the store.
//! Ignored unless SANDBOX_S3_ENDPOINT and SANDBOX_S3_BUCKET are set; see
//! docker/object-store-test.sh, which starts MinIO and runs these.

use object_store::aws::AmazonS3Builder;
use object_store::ObjectStore;
use sandbox_cas::tiered_store::CachePolicy;
use sandbox_cas::{BlobStore, DiskStore, PutOptions, TieredStore};
use sandbox_types::hash::ContentHash;
use std::path::PathBuf;
use std::sync::Arc;

fn remote() -> Arc<dyn ObjectStore> {
    let endpoint = std::env::var("SANDBOX_S3_ENDPOINT").expect("set SANDBOX_S3_ENDPOINT");
    let bucket = std::env::var("SANDBOX_S3_BUCKET").expect("set SANDBOX_S3_BUCKET");
    let store = AmazonS3Builder::from_env()
        .with_endpoint(endpoint.clone())
        .with_bucket_name(bucket)
        .with_allow_http(endpoint.starts_with("http://"))
        .with_virtual_hosted_style_request(false)
        .build()
        .expect("the object store opens");
    Arc::new(store)
}

/// A node: a local disk store in its own directory, with the shared remote behind it.
fn node(name: &str, remote: Arc<dyn ObjectStore>) -> (TieredStore, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sandbox-cas-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let local = DiskStore::new(dir.clone()).expect("the local store opens");
    (
        TieredStore::new(Arc::new(local), remote, CachePolicy::WriteThrough),
        dir,
    )
}

#[tokio::test]
#[ignore = "needs SANDBOX_S3_ENDPOINT and SANDBOX_S3_BUCKET; see docker/object-store-test.sh"]
async fn a_blob_written_on_one_node_is_read_on_another() {
    let remote = remote();
    let tag = format!("minio test {}", std::process::id());
    let data = tag.as_bytes();

    let (writer, writer_dir) = node("writer", remote.clone());
    let hash = writer.put(data, PutOptions::default()).await.unwrap();

    // The second node has nothing locally, so its read comes from the object store.
    let (reader, reader_dir) = node("reader", remote.clone());
    assert!(!reader.local_store().has(&hash).await.unwrap());
    let read = reader
        .get(&hash)
        .await
        .unwrap()
        .expect("the remote has the blob");
    assert_eq!(read.as_ref(), data);
    assert!(reader.has(&hash).await.unwrap());
    assert!(reader.list(None).await.unwrap().contains(&hash));

    // A delete on one node removes the blob for every node.
    assert!(reader.delete(&hash).await.unwrap());
    let (after, after_dir) = node("after", remote.clone());
    assert!(after.get(&ContentHash::new(data)).await.unwrap().is_none());

    for dir in [writer_dir, reader_dir, after_dir] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
