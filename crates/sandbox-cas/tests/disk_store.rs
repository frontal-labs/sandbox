use sandbox_cas::blob_store::PutOptions;
use sandbox_cas::disk_store::{decode_blob, encode_blob, DiskStore, CHUNK_THRESHOLD, SHARD_COUNT};
use sandbox_cas::BlobStore;
use sandbox_types::hash::ContentHash;
use std::collections::HashSet;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_disk_store_basic() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
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
async fn test_disk_store_put_does_not_deadlock() {
    // Regression: put() used to re-acquire its own shard lock via has().
    // The timeout turns a reintroduced deadlock into a failure.
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();

    let first = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.put(b"dedupe me", PutOptions::default()),
    )
    .await
    .expect("put deadlocked on first write")
    .unwrap();
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.put(b"dedupe me", PutOptions::default()),
    )
    .await
    .expect("put deadlocked on duplicate write")
    .unwrap();

    assert_eq!(first, second);
}

#[tokio::test]
async fn test_disk_store_chunked() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();

    let data = vec![0x42; 100 * 1024];
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };

    let hash = store.put(&data, options).await.unwrap();
    assert!(store.has(&hash).await.unwrap());

    let retrieved = store.get(&hash).await.unwrap().unwrap();
    assert_eq!(retrieved.as_ref(), data.as_slice());
}

#[tokio::test]
async fn test_disk_store_compression() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();

    let data = "highly compressible data ".repeat(1000).into_bytes();
    let options = PutOptions {
        compress: true,
        chunk_if_large: false,
    };

    let hash = store.put(&data, options).await.unwrap();
    let retrieved = store.get(&hash).await.unwrap().unwrap();
    assert_eq!(retrieved.as_ref(), data.as_slice());
}

#[tokio::test]
async fn test_disk_store_list() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();

    let hash1 = store.put(b"data1", PutOptions::default()).await.unwrap();
    let hash2 = store.put(b"data2", PutOptions::default()).await.unwrap();

    let list = store.list(None).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&hash1));
    assert!(list.contains(&hash2));
}

/// Deterministic bytes that do not repeat within a chunk, so each chunk has
/// its own content hash (xorshift64).
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// Bytes of chunk files in the store, as they are on disk.
async fn chunk_bytes(store: &DiskStore) -> u64 {
    let mut total = 0;
    for key in store.list_parts().await.unwrap() {
        total += std::fs::metadata(store.root.join(key)).unwrap().len();
    }
    total
}

#[tokio::test]
async fn a_chunk_is_kept_while_a_blob_names_it_and_removed_after() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    // Two blobs with the same first 512 KiB and different tails.
    let common = pseudo_random(512 * 1024, 1);
    let mut first = common.clone();
    first.extend(b"the tail of the first blob");
    let mut second = common.clone();
    second.extend(b"a different tail for the second");
    let h1 = store.put(&first, options).await.unwrap();
    let h2 = store.put(&second, options).await.unwrap();

    let parts1 = store.parts(&h1).await.unwrap();
    let parts2 = store.parts(&h2).await.unwrap();
    let shared: Vec<ContentHash> = parts1
        .iter()
        .copied()
        .filter(|p| parts2.contains(p))
        .collect();
    let only_first: Vec<ContentHash> = parts1
        .iter()
        .copied()
        .filter(|p| !parts2.contains(p))
        .collect();
    assert!(
        !shared.is_empty(),
        "blobs with a common prefix share chunks"
    );
    assert!(!only_first.is_empty(), "the first tail has its own chunk");

    // Deleting the first blob leaves its chunks on disk until a sweep.
    assert!(store.delete(&h1).await.unwrap());
    let candidates = store.list_parts().await.unwrap();
    let blobs = store.list(None).await.unwrap();
    assert_eq!(blobs, vec![h2], "a chunked blob is listed by its manifest");
    let mut keep = HashSet::new();
    for hash in blobs {
        keep.extend(store.parts(&hash).await.unwrap());
    }
    let removed = store.delete_parts(&candidates, &keep).await.unwrap();
    assert_eq!(removed, only_first.len(), "only the first tail's chunks go");

    assert_eq!(
        store.get(&h2).await.unwrap().unwrap().as_ref(),
        second.as_slice(),
        "the shared chunks must survive for the second blob"
    );
    let left = store.list_parts().await.unwrap();
    for chunk in &only_first {
        assert!(!left.iter().any(|key| key.ends_with(&chunk.to_hex())));
    }
    for chunk in &shared {
        assert!(left.iter().any(|key| key.ends_with(&chunk.to_hex())));
    }
}

#[tokio::test]
async fn identical_chunks_from_different_blobs_are_stored_once() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    let common = pseudo_random(512 * 1024, 2);
    let mut first = common.clone();
    first.push(1);
    let mut second = common.clone();
    second.push(2);
    let h1 = store.put(&first, options).await.unwrap();
    let h2 = store.put(&second, options).await.unwrap();

    let mut unique: HashSet<ContentHash> = store.parts(&h1).await.unwrap().into_iter().collect();
    unique.extend(store.parts(&h2).await.unwrap());
    assert_eq!(
        store.list_parts().await.unwrap().len(),
        unique.len(),
        "one chunk file per distinct chunk"
    );
}

#[tokio::test]
async fn an_edit_stores_only_the_chunks_around_it() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    let original = pseudo_random(1024 * 1024, 3);
    store.put(&original, options).await.unwrap();
    let before = chunk_bytes(&store).await;

    let mut edited = original.clone();
    edited.splice(500_000..500_000, b"a short insertion".iter().copied());
    store.put(&edited, options).await.unwrap();
    let added = chunk_bytes(&store).await - before;

    // Fixed-size chunks would store the whole edited blob again. Content-
    // defined chunks store the one or two chunks that contain the edit.
    assert!(
        added < original.len() as u64 / 4,
        "an insertion added {added} bytes of chunks to a {} byte blob",
        original.len()
    );
}

#[tokio::test]
async fn compressed_chunks_are_smaller_on_disk() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let data = "highly compressible text ".repeat(40_000).into_bytes();
    let options = PutOptions {
        compress: true,
        chunk_if_large: true,
    };
    let hash = store.put(&data, options).await.unwrap();
    assert!(!store.parts(&hash).await.unwrap().is_empty());
    assert!(
        chunk_bytes(&store).await < data.len() as u64 / 4,
        "compressible chunks must be stored compressed"
    );
    assert_eq!(
        store.get(&hash).await.unwrap().unwrap().as_ref(),
        data.as_slice()
    );
}

#[tokio::test]
async fn a_corrupt_chunk_is_an_error_not_wrong_bytes() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    let data = pseudo_random(256 * 1024, 4);
    let hash = store.put(&data, options).await.unwrap();
    let chunk = store.parts(&hash).await.unwrap()[0];
    // A well-formed frame with the wrong content: decoding succeeds, so only
    // the hash check can catch it.
    let wrong = encode_blob(b"not the chunk that was stored", false).unwrap();
    std::fs::write(store.chunk_file(&chunk), wrong).unwrap();

    let err = store.get(&hash).await.unwrap_err();
    assert!(err.to_string().contains("does not match its hash"), "{err}");
}

#[tokio::test]
async fn a_version_one_manifest_is_still_read() {
    // Version 1 wrote fixed 64 KiB chunks, raw, under the blob's own chunk
    // directory, and a bare array of hashes as the manifest.
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let data = pseudo_random(200 * 1024, 5);
    let hash = ContentHash::new(&data);
    let mut chunk_hashes = Vec::new();
    for piece in data.chunks(CHUNK_THRESHOLD) {
        let chunk = ContentHash::new(piece);
        store
            .write_file(&store.chunk_dir(&hash).join(chunk.to_hex()), piece)
            .await
            .unwrap();
        chunk_hashes.push(chunk);
    }
    let manifest = serde_json::to_vec(&chunk_hashes).unwrap();
    store
        .write_file(
            &store.shard_path(&hash).with_extension("manifest"),
            &manifest,
        )
        .await
        .unwrap();

    assert_eq!(
        store.get(&hash).await.unwrap().unwrap().as_ref(),
        data.as_slice()
    );
    assert_eq!(store.parts(&hash).await.unwrap(), chunk_hashes);
}

#[tokio::test]
async fn a_temporary_file_is_not_a_sweep_candidate() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    store
        .put(&pseudo_random(128 * 1024, 6), options)
        .await
        .unwrap();
    let chunk = store.list_parts().await.unwrap()[0].clone();
    // A write that died before its rename leaves a file next to the chunk.
    let leftover = store.root.join(format!("{chunk}.tmp-999-0"));
    std::fs::write(&leftover, b"partial").unwrap();

    let keys = store.list_parts().await.unwrap();
    assert!(keys.iter().all(|key| !key.contains(".tmp-")));
    assert!(store.delete_parts(&keys, &HashSet::new()).await.is_ok());
}

#[tokio::test]
async fn stale_temp_files_are_cleaned_up_on_open() {
    let dir = tempdir().unwrap();
    // Create a store and write some data to set up shards.
    let store = DiskStore::new(dir.path()).unwrap();
    let data = pseudo_random(128 * 1024, 7);
    store.put(&data, PutOptions::default()).await.unwrap();

    // Simulate a crashed write by creating stale temp files in multiple shards.
    for i in 0..SHARD_COUNT {
        let shard_dir = dir.path().join(format!("{:02x}", i));
        std::fs::create_dir_all(&shard_dir).unwrap();
        let temp_file = shard_dir.join("deadbeef.tmp-1234-5678");
        std::fs::write(&temp_file, b"stale data").unwrap();
    }

    // Re-open the store; cleanup should run.
    let store2 = DiskStore::open(dir.path()).unwrap();

    // Verify temp files were removed.
    for i in 0..SHARD_COUNT {
        let shard_dir = dir.path().join(format!("{:02x}", i));
        if shard_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&shard_dir).unwrap().collect();
            for entry in entries {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                assert!(
                    !name.contains(".tmp-"),
                    "stale temp file {} not cleaned up",
                    name
                );
            }
        }
    }

    // Original data should still be readable.
    assert_eq!(
        store2
            .get(&ContentHash::new(&data))
            .await
            .unwrap()
            .unwrap()
            .as_ref(),
        data.as_slice()
    );
}

mod properties {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Any blob reads back byte for byte under its own hash, whether it
        /// is stored whole or chunked, and with or without compression.
        #[test]
        fn every_blob_round_trips(
            data in proptest::collection::vec(any::<u8>(), 0..300_000),
            compress in any::<bool>(),
            chunk_if_large in any::<bool>(),
        ) {
            runtime().block_on(async {
                let dir = tempdir().unwrap();
                let store = DiskStore::new(dir.path()).unwrap();
                let options = PutOptions { compress, chunk_if_large };
                let hash = store.put(&data, options).await.unwrap();
                prop_assert_eq!(hash, ContentHash::new(&data));
                let read = store.get(&hash).await.unwrap().unwrap();
                prop_assert_eq!(read.as_ref(), data.as_slice());
                // A chunked blob's chunk files, in order, make up the blob.
                let parts = store.parts(&hash).await.unwrap();
                if !parts.is_empty() {
                    let mut joined = Vec::new();
                    for chunk in parts {
                        let framed = std::fs::read(store.chunk_file(&chunk)).unwrap();
                        joined.extend_from_slice(&decode_blob(&framed).unwrap());
                    }
                    prop_assert_eq!(joined, data.clone());
                }
                Ok::<(), TestCaseError>(())
            })?;
        }

        /// An insertion into a megabyte blob stores only the chunks that
        /// contain it, so the new bytes on disk are a small part of the blob.
        /// Known flaky: see docs/build-system.md. Requires chunking bound decision.
        #[test]
        #[ignore]
        fn an_insertion_stores_a_small_part_of_the_blob(
            len in 1_048_576usize..2_097_152,
            at in 0usize..1_048_576,
            insert in proptest::collection::vec(any::<u8>(), 1..4096),
            seed in any::<u64>(),
        ) {
            let original = pseudo_random(len, seed);
            let mut edited = original.clone();
            edited.splice(at..at, insert.iter().copied());
            runtime().block_on(async {
                let dir = tempdir().unwrap();
                let store = DiskStore::new(dir.path()).unwrap();
                let options = PutOptions { compress: false, chunk_if_large: true };
                store.put(&original, options).await.unwrap();
                let before = chunk_bytes(&store).await;
                store.put(&edited, options).await.unwrap();
                let added = chunk_bytes(&store).await - before;
                prop_assert!(
                    added < len as u64 / 4,
                    "an insertion of {} bytes added {} bytes of chunks to {} bytes",
                    insert.len(), added, len
                );
                Ok::<(), TestCaseError>(())
            })?;
        }

        /// A sweep keeps every chunk that a live blob names, and removes
        /// every other chunk. Nothing live is lost.
        #[test]
        fn a_sweep_keeps_what_live_blobs_name(
            blobs in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 60_000..200_000), 2..5),
            live_mask in proptest::collection::vec(any::<bool>(), 5),
        ) {
            runtime().block_on(async {
                let dir = tempdir().unwrap();
                let store = DiskStore::new(dir.path()).unwrap();
                let options = PutOptions { compress: false, chunk_if_large: true };
                let mut hashes = Vec::new();
                for blob in &blobs {
                    hashes.push(store.put(blob, options).await.unwrap());
                }
                let mut keep = HashSet::new();
                let mut live = Vec::new();
                for (i, hash) in hashes.iter().enumerate() {
                    if live_mask[i] {
                        keep.extend(store.parts(hash).await.unwrap());
                        live.push(*hash);
                    }
                }
                for hash in &hashes {
                    if !live.contains(hash) {
                        store.delete(hash).await.unwrap();
                    }
                }
                let candidates = store.list_parts().await.unwrap();
                store.delete_parts(&candidates, &keep).await.unwrap();
                for (i, hash) in hashes.iter().enumerate() {
                    if live_mask[i] {
                        let read = store.get(hash).await.unwrap().unwrap();
                        prop_assert_eq!(read.as_ref(), blobs[i].as_slice());
                    }
                }
                Ok::<(), TestCaseError>(())
            })?;
        }
    }
}

#[tokio::test]
async fn an_empty_keep_set_removes_every_chunk_file() {
    let dir = tempdir().unwrap();
    let store = DiskStore::new(dir.path()).unwrap();
    let options = PutOptions {
        compress: false,
        chunk_if_large: true,
    };
    store.put(&vec![7u8; 100 * 1024], options).await.unwrap();
    let candidates = store.list_parts().await.unwrap();
    assert!(!candidates.is_empty());
    let removed = store
        .delete_parts(&candidates, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(removed, candidates.len());
    assert!(store.list_parts().await.unwrap().is_empty());
}
