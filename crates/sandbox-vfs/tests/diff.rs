use sandbox_cas::{BlobStore, MemStore};
use sandbox_types::event::FileOp;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::ops::{OpenFlags, SessionFs};
use sandbox_vfs::{diff_trees, reachable_blobs, TreeChange, Vfs};
use std::path::Path;
use std::sync::Arc;

fn rw() -> OpenFlags {
    OpenFlags {
        read: true,
        write: true,
        create: true,
        ..OpenFlags::default()
    }
}

async fn write_file(vfs: &Vfs, path: &str, data: &[u8]) {
    let h = vfs.open(Path::new(path), rw()).await.unwrap();
    vfs.write(&h, 0, data).await.unwrap();
    vfs.close(&h);
}

async fn commit_of(vfs: &Vfs) -> ContentHash {
    vfs.checkpoint().await.unwrap()
}

fn paths(changes: &[TreeChange]) -> Vec<(&str, FileOp)> {
    changes.iter().map(|c| (c.path.as_str(), c.op)).collect()
}

#[tokio::test]
async fn identical_trees_have_no_changes() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/a.txt", b"a").await;
    let root = commit_of(&vfs).await;
    assert!(diff_trees(&*store, root, root).await.unwrap().is_empty());
}

#[tokio::test]
async fn creates_are_reported_with_their_directories() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    let before = commit_of(&vfs).await;
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/a.txt", b"hi").await;
    let after = commit_of(&vfs).await;

    let changes = diff_trees(&*store, before, after).await.unwrap();
    assert_eq!(
        paths(&changes),
        vec![("/d", FileOp::Mkdir), ("/d/a.txt", FileOp::Create)]
    );
    assert_eq!(changes[0].before, None);
    assert_eq!(changes[0].after, None);
    assert!(changes[1].after.is_some());
}

#[tokio::test]
async fn a_rewritten_file_carries_both_content_hashes() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/a.txt", b"one").await;
    write_file(&vfs, "/keep.txt", b"same").await;
    let before = commit_of(&vfs).await;
    write_file(&vfs, "/a.txt", b"two").await;
    let after = commit_of(&vfs).await;

    let changes = diff_trees(&*store, before, after).await.unwrap();
    assert_eq!(paths(&changes), vec![("/a.txt", FileOp::Write)]);
    assert!(changes[0].before.is_some());
    assert!(changes[0].after.is_some());
    assert_ne!(changes[0].before, changes[0].after);
}

#[tokio::test]
async fn a_removed_directory_reports_every_child() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/a.txt", b"a").await;
    vfs.mkdir(Path::new("/d/sub"), 0o755).await.unwrap();
    write_file(&vfs, "/d/sub/b.txt", b"b").await;
    let before = commit_of(&vfs).await;
    vfs.rm(Path::new("/d"), true).await.unwrap();
    let after = commit_of(&vfs).await;

    let changes = diff_trees(&*store, before, after).await.unwrap();
    assert_eq!(
        paths(&changes),
        vec![
            ("/d", FileOp::Delete),
            ("/d/a.txt", FileOp::Delete),
            ("/d/sub", FileOp::Delete),
            ("/d/sub/b.txt", FileOp::Delete),
        ]
    );
    assert!(changes[1].before.is_some());
    assert_eq!(changes[1].after, None);
}

#[tokio::test]
async fn a_file_replaced_by_a_directory_is_a_delete_then_a_create() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/x", b"file").await;
    let before = commit_of(&vfs).await;
    vfs.rm(Path::new("/x"), false).await.unwrap();
    vfs.mkdir(Path::new("/x"), 0o755).await.unwrap();
    let after = commit_of(&vfs).await;

    let changes = diff_trees(&*store, before, after).await.unwrap();
    assert_eq!(
        paths(&changes),
        vec![("/x", FileOp::Delete), ("/x", FileOp::Mkdir)]
    );
}

#[tokio::test]
async fn symlinks_are_created_and_retargeted() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/target", b"t").await;
    vfs.symlink(Path::new("/target"), Path::new("/link"))
        .await
        .unwrap();
    let first = commit_of(&vfs).await;
    vfs.rm(Path::new("/link"), false).await.unwrap();
    vfs.symlink(Path::new("/elsewhere"), Path::new("/link"))
        .await
        .unwrap();
    let second = commit_of(&vfs).await;

    let changes = diff_trees(&*store, first, second).await.unwrap();
    assert_eq!(paths(&changes), vec![("/link", FileOp::Symlink)]);
}

#[tokio::test]
async fn the_empty_tree_is_the_nil_root() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/a", b"a").await;
    let root = commit_of(&vfs).await;

    let changes = diff_trees(&*store, ContentHash::nil(), root).await.unwrap();
    assert_eq!(paths(&changes), vec![("/a", FileOp::Create)]);
}

#[tokio::test]
async fn a_missing_node_is_an_error_not_an_empty_tree() {
    let store = MemStore::new();
    let missing = ContentHash::new(b"absent");
    assert!(diff_trees(&store, ContentHash::nil(), missing)
        .await
        .is_err());
}

#[tokio::test]
async fn reachable_blobs_cover_nodes_and_contents_but_not_strangers() {
    let store = Arc::new(MemStore::new());
    let vfs = Vfs::new(ContentHash::nil(), store.clone());
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/a.txt", b"kept").await;
    let root = commit_of(&vfs).await;
    let stranger = store
        .put(b"not part of any tree", sandbox_cas::PutOptions::default())
        .await
        .unwrap();

    let live = reachable_blobs(&*store, &[root]).await.unwrap();
    // The root, /d, and a.txt's node, plus a.txt's contents.
    assert!(live.contains(&root));
    assert!(live.contains(&ContentHash::new(b"kept")));
    assert!(!live.contains(&stranger));
    assert_eq!(live.len(), 4);
}

#[tokio::test]
async fn reachable_blobs_of_the_nil_root_are_empty_and_a_missing_node_is_an_error() {
    let store = MemStore::new();
    assert!(reachable_blobs(&store, &[ContentHash::nil()])
        .await
        .unwrap()
        .is_empty());
    assert!(reachable_blobs(&store, &[ContentHash::new(b"absent")])
        .await
        .is_err());
}
