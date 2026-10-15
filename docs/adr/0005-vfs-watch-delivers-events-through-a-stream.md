# 0005: VFS watch delivers events through a stream

## Status
Accepted

## Date
2026-10-15

## Context
`SessionFs::watch` returned a `WatchHandle(u64)` and was not implemented: it returned `Unsupported`. A handle with no way to receive events is useless, so the method could not be implemented as declared. The VFS is in-process and is the only writer of its overlay, so change notifications can come from the VFS itself, with no host file system involved.

## Decision
- `SessionFs::watch(path)` returns a `Watch`. `Watch::next().await` yields the next `WatchEvent` under the watched path, or `None` once the VFS is gone.
- `WatchEvent` is `Created(path)`, `Modified(path)`, `Removed(path)`, `Renamed { from, to }`, or `Overflow`. Paths are normalized absolute paths, as the VFS stores them.
- A watch covers its path and everything under it. A `Renamed` event is delivered to a watch when either side is under the watched path.
- Every mutating operation emits events after it changes the overlay: `open` with create emits `Created`, `write`, `truncate`, and `open` with truncate emit `Modified`, `mkdir` and `symlink` emit `Created`, `rm` emits `Removed` for each removed path, and `rename` emits one `Renamed` per moved node. A rename therefore reaches a watch placed inside the moved subtree.
- Events go through one `tokio::sync::broadcast` channel per VFS, with 1024 slots. A reader that falls behind gets `Overflow` in place of the events it missed. It should re-read the tree. An `Overflow` is delivered whatever path the watch covers.
- `WatchHandle` is removed. There is no separate unwatch: dropping the `Watch` ends the subscription.
- Events describe changes to the overlay. They are not raised when the committed root is rewritten by `commit`, because `commit` does not change any file's contents.

## Consequences
### Positive
- Watching needs no host file system and no polling, and it works the same on every platform.
- A slow watcher cannot block writers or grow memory without bound. It gets `Overflow` instead.

### Negative
- This is a breaking change to a public trait. The only implementer is `Vfs`, and no caller used `WatchHandle`.
- Events are not persisted. A watcher that is not running when a change happens never sees it.
- The server does not expose watch yet. The HTTP API and the SDKs do not carry events. Watching is available to in-process callers through `Session::fs()`.
