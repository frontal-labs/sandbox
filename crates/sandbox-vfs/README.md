# Sandbox VFS

Virtual filesystem for the sandbox system.

## Status

Working in memory, with a content-addressed commit:

- `read`, `write`, `open` (with `create`, `exclusive`, `truncate`, `append`), `stat`, `readdir`, `mkdir`, `rm` (with `recursive`), `rename` (whole subtrees), `symlink`, `readlink`, and `truncate` operate on the overlay.
- Symlinks are followed in every path component, and in the final component for `open`, `stat`, `readdir`, and `truncate`. `readlink`, `rm`, `rename`, and `symlink` act on the link itself. Resolution is capped at 40 hops.
- `commit` persists the tree as a Merkle directory tree and returns the root hash. `Vfs::new(root, store)` restores it. Commit is incremental: only directories on changed paths are rewritten, and untouched subtrees keep their hashes.
- Quotas `total_bytes`, `max_file_size`, `max_path_depth`, and `inode_count` are enforced on create and write. Committed totals are cached, so a quota check costs O(changes), not O(tree).
- `watch` returns `VfsError::Unsupported`.

Known limits:

- File modes and timestamps are not stored. Directories report mode `0o755` and files `0o644`, and `modified` is `0`.
- Quota checks are not reserved, so concurrent writers can overshoot a limit.
- Overlay writes are not coalesced. A large file is held in memory until commit.
- Not implemented: `export_ext4` and `import_from_dir` (PLAN.md section 4.3), and a model-based fuzz target for the path normalizer.
- Symlink targets containing `..` are rejected, so relative links cannot climb out of their directory.

## Overview

Copy-on-write filesystem: an immutable base tree addressed by a `ContentHash` (read through `sandbox-cas`'s `BlobStore`) plus a mutable in-memory overlay. Deletes are whiteouts.

## Public API (from `src/lib.rs`)

- `Vfs::new(root: ContentHash, store: Arc<dyn BlobStore>)` and `with_quotas(Quotas)`.
- `SessionFs` trait (async): `open`, `read`, `write`, `stat`, `readdir`, `mkdir`, `rm`, `rename`, `symlink`, `readlink`, `truncate`, `watch`, `commit`.
- Types: `OpenFlags`, `FileHandle`, `FileStat`, `DirEntry`, `WatchHandle`, `WatchEvent`.
- `Quotas` (`quota.rs`); `Node` (overlay) and `Stored` (committed index) (`node.rs`).
- `VfsError`, `VfsResult` (`error.rs`).

## Invariants and security assumptions

- Paths are normalized to a leading `/`. NUL bytes and `..` components are rejected, so no path can escape the root.
- Writes require an existing parent directory. Removing a non-empty directory requires `recursive`.
- Committed nodes are JSON in the blob store. A node's hash covers its whole subtree.
- The VFS must never expose host paths; it is the only filesystem view for session code (see `sandbox-wasm`).

## Example

```rust
let store: Arc<dyn BlobStore> = Arc::new(MemStore::new());
let mut vfs = Vfs::new(ContentHash::nil(), store.clone());
let h = vfs.open(Path::new("/note.txt"), OpenFlags { read: true, write: true, create: true, ..Default::default() }).await?;
vfs.write(&h, 0, b"hi").await?;
let root = vfs.commit().await?;           // persist; restore later with Vfs::new(root, store)
```

See `PLAN.md` section 4.3 for the target spec.
