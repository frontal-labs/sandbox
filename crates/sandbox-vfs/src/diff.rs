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
        if pending.before != pending.after {
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
        if hash != ContentHash::nil() && live.insert(hash) {
            match load(store, hash).await? {
                Stored::Directory { entries } => pending.extend(entries.values().copied()),
                Stored::File { content, .. } => {
                    live.insert(content);
                }
                Stored::Symlink(_) => {}
            }
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
