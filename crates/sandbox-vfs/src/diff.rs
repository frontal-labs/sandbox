//! Differences between two committed trees. See PLAN.md §4.12 (`diff(from_root, to_root)`).
//!
//! Committed directories are Merkle objects, so a directory whose hash is the
//! same in both trees is skipped without being read. The walk reads only the
//! subtrees that changed.

use crate::error::{VfsError, VfsResult};
use crate::node::Stored;
use sandbox_cas::BlobStore;
use sandbox_types::event::FileOp;
use sandbox_types::hash::ContentHash;
use std::collections::{BTreeSet, HashSet};

/// One path that differs between two committed trees.
///
/// `before` and `after` are content hashes of regular files. They are `None`
/// for directories and symlinks, and for the side that does not exist.
/// Permission bits are not part of a committed tree, so a chmod is not reported.
/// A rename appears as a `Delete` and a `Create`, because the tree stores no
/// identity for a file that would tie the two together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeChange {
    pub path: String,
    pub op: FileOp,
    pub before: Option<ContentHash>,
    pub after: Option<ContentHash>,
}

/// A pending comparison of one path. Either side is `None` when the path does
/// not exist there.
struct Pending {
    path: String,
    before: Option<ContentHash>,
    after: Option<ContentHash>,
}

/// Lists every path that differs between the committed trees `from` and `to`,
/// sorted by path. A root of [`ContentHash::nil`] is the empty tree.
///
/// Both roots must be in `store`. A missing node is an error rather than a
/// silently empty tree, because an empty tree would report the whole tree as
/// created or deleted.
pub async fn diff_trees(
    store: &dyn BlobStore,
    from: ContentHash,
    to: ContentHash,
) -> VfsResult<Vec<TreeChange>> {
    let mut changes = Vec::new();
    let mut stack = vec![Pending {
        path: "/".to_string(),
        before: Some(from),
        after: Some(to),
    }];

    while let Some(pending) = stack.pop() {
        if pending.before == pending.after {
            continue;
        }
        let before = match pending.before {
            Some(hash) => Some(load(store, hash).await?),
            None => None,
        };
        let after = match pending.after {
            Some(hash) => Some(load(store, hash).await?),
            None => None,
        };
        compare(&pending, before, after, &mut changes, &mut stack);
    }

    // The stack visits siblings in reverse. Sorting makes the order depend on
    // the paths only. The sort is stable, so a path whose kind changed keeps its
    // `Delete` before its `Create`.
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// Compares one path and records its changes. Children that need their own
/// comparison are pushed onto `stack`.
fn compare(
    pending: &Pending,
    before: Option<Stored>,
    after: Option<Stored>,
    changes: &mut Vec<TreeChange>,
    stack: &mut Vec<Pending>,
) {
    let path = &pending.path;
    match (before, after) {
        (None, None) => {}
        (Some(old), None) => {
            remove(path, &old, changes, stack);
        }
        (None, Some(new)) => {
            create(path, &new, changes, stack);
        }
        (Some(old), Some(new)) => match (old, new) {
            (Stored::Directory { entries: old }, Stored::Directory { entries: new }) => {
                let names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
                for name in names {
                    stack.push(Pending {
                        path: child_path(path, name),
                        before: old.get(name).copied(),
                        after: new.get(name).copied(),
                    });
                }
            }
            (
                Stored::File {
                    content: old_content,
                    ..
                },
                Stored::File {
                    content: new_content,
                    ..
                },
            ) => {
                changes.push(TreeChange {
                    path: path.clone(),
                    op: FileOp::Write,
                    before: Some(old_content),
                    after: Some(new_content),
                });
            }
            (Stored::Symlink(old), Stored::Symlink(new)) => {
                if old != new {
                    changes.push(TreeChange {
                        path: path.clone(),
                        op: FileOp::Symlink,
                        before: None,
                        after: None,
                    });
                }
            }
            (old, new) => {
                // The kind changed, for example a file replaced by a directory.
                // Report the removal, then the creation.
                remove(path, &old, changes, stack);
                create(path, &new, changes, stack);
            }
        },
    }
}

/// Records the removal of the node at `path`. A directory's children are
/// removed too, so each one is reported.
fn remove(path: &str, node: &Stored, changes: &mut Vec<TreeChange>, stack: &mut Vec<Pending>) {
    changes.push(TreeChange {
        path: path.to_string(),
        op: FileOp::Delete,
        before: file_content(node),
        after: None,
    });
    if let Stored::Directory { entries } = node {
        for (name, child) in entries {
            stack.push(Pending {
                path: child_path(path, name),
                before: Some(*child),
                after: None,
            });
        }
    }
}

/// Records the creation of the node at `path`. A directory's children are
/// created too, so each one is reported.
fn create(path: &str, node: &Stored, changes: &mut Vec<TreeChange>, stack: &mut Vec<Pending>) {
    let op = match node {
        Stored::Directory { .. } => FileOp::Mkdir,
        Stored::File { .. } => FileOp::Create,
        Stored::Symlink(_) => FileOp::Symlink,
    };
    changes.push(TreeChange {
        path: path.to_string(),
        op,
        before: None,
        after: file_content(node),
    });
    if let Stored::Directory { entries } = node {
        for (name, child) in entries {
            stack.push(Pending {
                path: child_path(path, name),
                before: None,
                after: Some(*child),
            });
        }
    }
}

fn file_content(node: &Stored) -> Option<ContentHash> {
    match node {
        Stored::File { content, .. } => Some(*content),
        Stored::Directory { .. } | Stored::Symlink(_) => None,
    }
}

fn child_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Every blob the committed trees at `roots` need: their node objects and their
/// file contents. This is the mark step of garbage collection. The nil root is
/// the empty tree and needs no blobs. A node that cannot be read is an error,
/// so a damaged tree is never treated as empty.
pub async fn reachable_blobs(
    store: &dyn BlobStore,
    roots: &[ContentHash],
) -> VfsResult<HashSet<ContentHash>> {
    let mut live = HashSet::new();
    let mut pending: Vec<ContentHash> = roots.to_vec();
    while let Some(hash) = pending.pop() {
        if hash == ContentHash::nil() || !live.insert(hash) {
            continue;
        }
        match load(store, hash).await? {
            Stored::Directory { entries } => pending.extend(entries.values().copied()),
            Stored::File { content, .. } => {
                live.insert(content);
            }
            Stored::Symlink(_) => {}
        }
    }
    Ok(live)
}

/// Reads a committed node. The nil hash is the empty root directory, as in
/// `Vfs::root_node`.
pub(crate) async fn load(store: &dyn BlobStore, hash: ContentHash) -> VfsResult<Stored> {
    if hash == ContentHash::nil() {
        return Ok(Stored::Directory {
            entries: Default::default(),
        });
    }
    let data = store
        .get(&hash)
        .await?
        .ok_or_else(|| VfsError::NotFound(format!("node {hash}")))?;
    serde_json::from_slice(&data).map_err(|e| VfsError::Serialization(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpenFlags, SessionFs};
    use crate::vfs::Vfs;
    use sandbox_cas::MemStore;
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
}
