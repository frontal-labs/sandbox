# sandbox-vfs

Virtual filesystem for the sandbox system.

## Status

Partial. The `SessionFs` trait (`src/ops.rs`) is implemented for `Vfs` (`src/vfs.rs`), but key operations are stubs:

- `read` returns empty bytes and `write` returns `0`, so file content is not yet stored or returned.
- `readdir` reports every entry as a file with size 0.
- `watch` registers a channel but no events are delivered.
- `commit` flushes dirty paths; its completeness is unverified.
- Quotas are stored by `with_quotas` but not enforced (the usage tracker is unused).
- `rm` is non-recursive in effect and does not check the base layer.
- `export_ext4` and `import_from_dir` from PLAN.md are not implemented.
- No tests exist yet.

## Overview

Copy-on-write filesystem: an immutable base tree addressed by a `ContentHash` (read through `sandbox-cas`'s `BlobStore`) plus a mutable in-memory overlay. Deletes are whiteouts.

## Public API (from `src/lib.rs`)

- `Vfs::new(root: ContentHash, store: Arc<dyn BlobStore>)` and `with_quotas(Quotas)`.
- `SessionFs` trait (async): `open`, `read`, `write`, `stat`, `readdir`, `mkdir`, `rm`, `rename`, `symlink`, `readlink`, `truncate`, `watch`, `commit`.
- Types: `OpenFlags`, `FileHandle`, `FileStat`, `DirEntry`, `WatchHandle`, `WatchEvent`.
- `Quotas`, `Usage`, `QuotaExceeded` (`quota.rs`); `Layer`, `Directory`, `FileNode`, `Node` (`layer.rs`, `node.rs`).
- `VfsError`, `VfsResult` (`error.rs`).

## Invariants and security assumptions

- Paths are normalized to a leading `/` and NUL bytes are rejected. `..` components are not specially handled yet, so escape-above-root safety is unverified.
- Symlink resolution depth cap is not yet implemented.
- The VFS must never expose host paths; it is the only filesystem view for session code (see `sandbox-wasm`).

## Example

Omitted: constructing a `Vfs` requires a `BlobStore`, and the read/write path is not functional yet.

See `PLAN.md` section 4.3 for the target spec.
