use super::*;
use crate::node::Stored;
use crate::quota::Quotas;
use sandbox_cas::MemStore;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn new_vfs() -> Vfs {
    Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()))
}

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
}

async fn read_file(vfs: &Vfs, path: &str) -> Vec<u8> {
    let h = vfs
        .open(Path::new(path), OpenFlags::default())
        .await
        .unwrap();
    vfs.read(&h, 0, usize::MAX).await.unwrap().to_vec()
}

#[tokio::test]
async fn write_then_read_round_trips() {
    let vfs = new_vfs();
    write_file(&vfs, "/a.txt", b"hello").await;
    assert_eq!(read_file(&vfs, "/a.txt").await, b"hello");
}

#[tokio::test]
async fn write_past_end_zero_fills_the_gap() {
    let vfs = new_vfs();
    let h = vfs.open(Path::new("/f"), rw()).await.unwrap();
    vfs.write(&h, 4, b"xy").await.unwrap();
    assert_eq!(read_file(&vfs, "/f").await, b"\0\0\0\0xy");
}

#[tokio::test]
async fn append_flag_writes_at_end() {
    let vfs = new_vfs();
    write_file(&vfs, "/log", b"one ").await;
    let flags = OpenFlags {
        read: false,
        write: true,
        append: true,
        ..OpenFlags::default()
    };
    let h = vfs.open(Path::new("/log"), flags).await.unwrap();
    vfs.write(&h, 0, b"two").await.unwrap();
    assert_eq!(read_file(&vfs, "/log").await, b"one two");
}

#[tokio::test]
async fn handle_access_mode_is_enforced() {
    let vfs = new_vfs();
    write_file(&vfs, "/ro", b"data").await;
    let ro = vfs
        .open(Path::new("/ro"), OpenFlags::default())
        .await
        .unwrap();
    assert!(matches!(
        vfs.write(&ro, 0, b"x").await,
        Err(VfsError::PermissionDenied(_))
    ));
}

#[tokio::test]
async fn readdir_lists_direct_children_only_in_order() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/d/sub"), 0o755).await.unwrap();
    write_file(&vfs, "/d/sub/deep", b"x").await;
    write_file(&vfs, "/d/b", b"bb").await;
    write_file(&vfs, "/d/a", b"a").await;

    let names: Vec<_> = vfs
        .readdir(Path::new("/d"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.is_dir, e.size))
        .collect();
    assert_eq!(
        names,
        vec![
            ("a".to_string(), false, 1),
            ("b".to_string(), false, 2),
            ("sub".to_string(), true, 0),
        ]
    );
}

#[tokio::test]
async fn writes_require_an_existing_parent_directory() {
    let vfs = new_vfs();
    let err = vfs
        .open(Path::new("/missing/file"), rw())
        .await
        .unwrap_err();
    assert!(matches!(err, VfsError::NotFound(_)));
}

#[tokio::test]
async fn rm_refuses_non_empty_directory_without_recursive() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/f", b"x").await;
    assert!(vfs.rm(Path::new("/d"), false).await.is_err());
    vfs.rm(Path::new("/d"), true).await.unwrap();
    assert!(matches!(
        vfs.stat(Path::new("/d")).await,
        Err(VfsError::NotFound(_))
    ));
    assert!(matches!(
        vfs.stat(Path::new("/d/f")).await,
        Err(VfsError::NotFound(_))
    ));
}

#[tokio::test]
async fn rename_moves_a_whole_subtree() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/src"), 0o755).await.unwrap();
    write_file(&vfs, "/src/x", b"payload").await;
    vfs.rename(Path::new("/src"), Path::new("/dst"))
        .await
        .unwrap();

    assert_eq!(read_file(&vfs, "/dst/x").await, b"payload");
    assert!(matches!(
        vfs.stat(Path::new("/src")).await,
        Err(VfsError::NotFound(_))
    ));
    assert!(matches!(
        vfs.stat(Path::new("/src/x")).await,
        Err(VfsError::NotFound(_))
    ));
}

#[tokio::test]
async fn rename_cannot_move_a_directory_into_itself() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/a"), 0o755).await.unwrap();
    assert!(vfs
        .rename(Path::new("/a"), Path::new("/a/b"))
        .await
        .is_err());
}

#[tokio::test]
async fn dot_dot_and_nul_paths_are_rejected() {
    let vfs = new_vfs();
    assert!(matches!(
        vfs.stat(Path::new("/a/../b")).await,
        Err(VfsError::InvalidPath(_))
    ));
    assert!(matches!(
        vfs.stat(&PathBuf::from("/a\0b")).await,
        Err(VfsError::InvalidPath(_))
    ));
}

#[tokio::test]
async fn symlinks_store_and_read_back_their_target() {
    let vfs = new_vfs();
    vfs.symlink(Path::new("/target"), Path::new("/link"))
        .await
        .unwrap();
    assert_eq!(vfs.readlink(Path::new("/link")).await.unwrap(), "/target");
    // Dangling: stat follows the link, so the missing target is reported.
    assert!(matches!(
        vfs.stat(Path::new("/link")).await,
        Err(VfsError::NotFound(_))
    ));
}

#[tokio::test]
async fn stat_and_read_follow_symlinks_to_files() {
    let vfs = new_vfs();
    write_file(&vfs, "/real.txt", b"through the link").await;
    vfs.symlink(Path::new("/real.txt"), Path::new("/ln"))
        .await
        .unwrap();
    let st = vfs.stat(Path::new("/ln")).await.unwrap();
    assert!(st.is_file && !st.is_symlink);
    assert_eq!(read_file(&vfs, "/ln").await, b"through the link");
}

#[tokio::test]
async fn relative_symlink_resolves_against_its_parent() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/data", b"rel").await;
    vfs.symlink(Path::new("data"), Path::new("/d/alias"))
        .await
        .unwrap();
    assert_eq!(read_file(&vfs, "/d/alias").await, b"rel");
}

#[tokio::test]
async fn symlinked_directory_in_the_middle_of_a_path() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/real"), 0o755).await.unwrap();
    write_file(&vfs, "/real/f", b"inside").await;
    vfs.symlink(Path::new("/real"), Path::new("/shortcut"))
        .await
        .unwrap();
    assert_eq!(read_file(&vfs, "/shortcut/f").await, b"inside");
}

#[tokio::test]
async fn symlink_loop_is_rejected() {
    let vfs = new_vfs();
    vfs.symlink(Path::new("/b"), Path::new("/a")).await.unwrap();
    vfs.symlink(Path::new("/a"), Path::new("/b")).await.unwrap();
    assert!(matches!(
        vfs.stat(Path::new("/a")).await,
        Err(VfsError::InvalidPath(_))
    ));
}

#[tokio::test]
async fn writing_through_a_dangling_link_creates_the_target() {
    let vfs = new_vfs();
    vfs.symlink(Path::new("/made"), Path::new("/ln"))
        .await
        .unwrap();
    write_file(&vfs, "/ln", b"created").await;
    assert_eq!(read_file(&vfs, "/made").await, b"created");
}

#[tokio::test]
async fn rm_and_rename_act_on_the_link_not_its_target() {
    let vfs = new_vfs();
    write_file(&vfs, "/target", b"keep me").await;
    vfs.symlink(Path::new("/target"), Path::new("/ln"))
        .await
        .unwrap();
    vfs.rename(Path::new("/ln"), Path::new("/ln2"))
        .await
        .unwrap();
    assert_eq!(vfs.readlink(Path::new("/ln2")).await.unwrap(), "/target");
    vfs.rm(Path::new("/ln2"), false).await.unwrap();
    assert_eq!(read_file(&vfs, "/target").await, b"keep me");
}

#[tokio::test]
async fn watch_reports_changes_under_its_path_only() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/dx"), 0o755).await.unwrap();
    let mut watch = vfs.watch(Path::new("/d")).await.unwrap();

    // /dx shares the text prefix "/d" but is a different directory, and /d is
    // the watched path, so nothing below /dx may appear.
    write_file(&vfs, "/dx/y", b"no").await;
    write_file(&vfs, "/d/a", b"1").await;
    vfs.rm(Path::new("/d/a"), false).await.unwrap();

    assert_eq!(watch.next().await, Some(WatchEvent::Created("/d/a".into())));
    assert_eq!(
        watch.next().await,
        Some(WatchEvent::Modified("/d/a".into()))
    );
    assert_eq!(watch.next().await, Some(WatchEvent::Removed("/d/a".into())));
}

#[tokio::test]
async fn rename_reaches_watches_on_both_sides_and_inside_the_subtree() {
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/a"), 0o755).await.unwrap();
    write_file(&vfs, "/a/f", b"x").await;
    let mut inside = vfs.watch(Path::new("/a/f")).await.unwrap();
    let mut destination = vfs.watch(Path::new("/b")).await.unwrap();

    vfs.rename(Path::new("/a"), Path::new("/b")).await.unwrap();

    // The moved nodes are reported in path order: /a, then /a/f.
    assert_eq!(
        inside.next().await,
        Some(WatchEvent::Renamed {
            from: "/a/f".into(),
            to: "/b/f".into()
        })
    );
    assert_eq!(
        destination.next().await,
        Some(WatchEvent::Renamed {
            from: "/a".into(),
            to: "/b".into()
        })
    );
    assert_eq!(
        destination.next().await,
        Some(WatchEvent::Renamed {
            from: "/a/f".into(),
            to: "/b/f".into()
        })
    );
}

#[tokio::test]
async fn a_watcher_that_falls_behind_gets_overflow() {
    let vfs = new_vfs();
    let mut watch = vfs.watch(Path::new("/")).await.unwrap();
    write_file(&vfs, "/f", b"").await;
    let h = vfs.open(Path::new("/f"), rw()).await.unwrap();
    for _ in 0..(EVENT_CAPACITY + 100) {
        vfs.write(&h, 0, b"x").await.unwrap();
    }

    assert_eq!(watch.next().await, Some(WatchEvent::Overflow));
    // After the overflow, later changes are delivered again.
    vfs.rm(Path::new("/f"), false).await.unwrap();
    let mut seen_removal = false;
    while let Some(event) = watch.next().await {
        if event == WatchEvent::Removed("/f".into()) {
            seen_removal = true;
            break;
        }
    }
    assert!(seen_removal);
}

#[tokio::test]
async fn watch_ends_when_the_vfs_is_dropped() {
    let vfs = new_vfs();
    let mut watch = vfs.watch(Path::new("/")).await.unwrap();
    drop(vfs);
    assert_eq!(watch.next().await, None);
}

#[tokio::test]
async fn file_size_quota_is_enforced() {
    let quotas = Quotas {
        max_file_size: 8,
        ..Quotas::default()
    };
    let vfs = new_vfs().with_quotas(quotas);
    let h = vfs.open(Path::new("/big"), rw()).await.unwrap();
    assert!(matches!(
        vfs.write(&h, 0, &[0u8; 9]).await,
        Err(VfsError::QuotaExceeded(_))
    ));
    assert_eq!(vfs.write(&h, 0, &[1u8; 8]).await.unwrap(), 8);
}

#[tokio::test]
async fn commit_then_restore_from_root_sees_the_same_tree() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
    vfs.mkdir(Path::new("/dir"), 0o755).await.unwrap();
    write_file(&vfs, "/dir/file", b"persisted").await;
    vfs.symlink(Path::new("/dir/file"), Path::new("/ln"))
        .await
        .unwrap();
    let root = vfs.commit().await.unwrap();

    let restored = Vfs::new(root, store);
    assert_eq!(read_file(&restored, "/dir/file").await, b"persisted");
    assert_eq!(
        restored.readlink(Path::new("/ln")).await.unwrap(),
        "/dir/file"
    );
    let names: Vec<_> = restored
        .readdir(Path::new("/dir"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["file".to_string()]);
}

#[tokio::test]
async fn deletes_after_commit_hide_committed_files() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/gone", b"x").await;
    let first = vfs.commit().await.unwrap();

    vfs.rm(Path::new("/gone"), false).await.unwrap();
    let second = vfs.commit().await.unwrap();

    assert_ne!(first, second);
    let restored = Vfs::new(second, store);
    assert!(matches!(
        restored.stat(Path::new("/gone")).await,
        Err(VfsError::NotFound(_))
    ));
}

#[tokio::test]
async fn committed_file_is_copied_on_write() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
    write_file(&vfs, "/f", b"original").await;
    let base = vfs.commit().await.unwrap();

    // Longer than the original, so the whole file is replaced.
    write_file(&vfs, "/f", b"OVERWRITTEN").await;
    assert_eq!(read_file(&vfs, "/f").await, b"OVERWRITTEN");

    // The committed root still holds the original bytes.
    let snapshot = Vfs::new(base, store);
    assert_eq!(read_file(&snapshot, "/f").await, b"original");
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Random writes at random offsets match a plain `Vec<u8>` model.
        #[test]
        fn writes_match_a_vec_model(
            ops in proptest::collection::vec(
                (0usize..256, proptest::collection::vec(any::<u8>(), 0..64)),
                1..20,
            )
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let vfs = new_vfs();
                let h = vfs.open(Path::new("/m"), rw()).await.unwrap();
                let mut model: Vec<u8> = Vec::new();

                for (offset, data) in ops {
                    vfs.write(&h, offset as u64, &data).await.unwrap();
                    if model.len() < offset + data.len() {
                        model.resize(offset + data.len(), 0);
                    }
                    model[offset..offset + data.len()].copy_from_slice(&data);
                }

                prop_assert_eq!(read_file(&vfs, "/m").await, model.clone());
                Ok(())
            })?;
        }

        /// Commit and restore preserves exactly what was written.
        #[test]
        fn commit_restore_round_trips(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
                let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
                write_file(&vfs, "/p", &data).await;
                let root = vfs.commit().await.unwrap();
                let restored = Vfs::new(root, store);
                prop_assert_eq!(read_file(&restored, "/p").await, data.clone());
                Ok(())
            })?;
        }
    }
}

#[tokio::test]
async fn inode_count_quota_is_enforced() {
    let quotas = Quotas {
        inode_count: 2,
        ..Quotas::default()
    };
    let vfs = new_vfs().with_quotas(quotas);
    vfs.mkdir(Path::new("/a"), 0o755).await.unwrap();
    write_file(&vfs, "/a/f", b"x").await;
    // Third inode would exceed the limit.
    assert!(matches!(
        vfs.mkdir(Path::new("/b"), 0o755).await,
        Err(VfsError::QuotaExceeded(_))
    ));
    assert!(matches!(
        vfs.open(Path::new("/c"), rw()).await,
        Err(VfsError::QuotaExceeded(_))
    ));
    // Writing to an existing file creates no inode.
    write_file(&vfs, "/a/f", b"y").await;
}

async fn root_entries(
    store: &Arc<dyn BlobStore>,
    root: ContentHash,
) -> BTreeMap<String, ContentHash> {
    let data = store.get(&root).await.unwrap().unwrap();
    match serde_json::from_slice::<Stored>(&data).unwrap() {
        Stored::Directory { entries } => entries,
        other => panic!("root is not a directory: {other:?}"),
    }
}

#[tokio::test]
async fn commit_reuses_hashes_of_untouched_subtrees() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
    vfs.mkdir(Path::new("/x"), 0o755).await.unwrap();
    write_file(&vfs, "/x/y", b"untouched").await;
    write_file(&vfs, "/z", b"before").await;
    let first = vfs.commit().await.unwrap();

    write_file(&vfs, "/z", b"after!").await;
    let second = vfs.commit().await.unwrap();

    let before = root_entries(&store, first).await;
    let after = root_entries(&store, second).await;
    assert_eq!(
        before["x"], after["x"],
        "untouched subtree must keep its hash"
    );
    assert_ne!(before["z"], after["z"]);
}

#[tokio::test]
async fn noop_commit_returns_the_same_root() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store);
    write_file(&vfs, "/a", b"1").await;
    let first = vfs.commit().await.unwrap();
    assert_eq!(vfs.commit().await.unwrap(), first);
}

#[tokio::test]
async fn recreated_directory_hides_its_old_committed_children() {
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    write_file(&vfs, "/d/old", b"gone").await;
    vfs.commit().await.unwrap();

    vfs.rm(Path::new("/d"), true).await.unwrap();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    let names: Vec<_> = vfs
        .readdir(Path::new("/d"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.is_empty(), "uncommitted view leaked: {names:?}");

    let root = vfs.commit().await.unwrap();
    let restored = Vfs::new(root, store);
    assert!(restored.readdir(Path::new("/d")).await.unwrap().is_empty());
    assert!(matches!(
        restored.stat(Path::new("/d/old")).await,
        Err(VfsError::NotFound(_))
    ));
}

#[tokio::test]
async fn quota_totals_stay_correct_across_commits() {
    let quotas = Quotas {
        total_bytes: 20,
        ..Quotas::default()
    };
    let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
    let mut vfs = Vfs::new(ContentHash::nil(), store).with_quotas(quotas);
    write_file(&vfs, "/a", b"0123456789").await;
    vfs.commit().await.unwrap();
    write_file(&vfs, "/b", b"0123456789").await;
    vfs.commit().await.unwrap();

    // 20 bytes are in use, so one more byte must be refused.
    let h = vfs.open(Path::new("/c"), rw()).await.unwrap();
    assert!(matches!(
        vfs.write(&h, 0, b"x").await,
        Err(VfsError::QuotaExceeded(_))
    ));
    // Overwriting an existing file within its size is still allowed.
    write_file(&vfs, "/a", b"abcdefghij").await;
}

#[tokio::test]
async fn closed_handle_is_rejected_and_handles_do_not_leak() {
    let vfs = new_vfs();
    let h = vfs.open(Path::new("/f"), rw()).await.unwrap();
    vfs.close(&h);
    assert!(matches!(
        vfs.write(&h, 0, b"x").await,
        Err(VfsError::InvalidPath(_))
    ));
    assert!(vfs.handles.read().is_empty());
}
