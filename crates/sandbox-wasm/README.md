# Sandbox WASM

WASI preview1 host on `wasmtime` for command modules, with files backed by the VFS.

## Status

Working. Covered by unit tests in `src/wasi.rs` and `src/registry.rs`, and by `tests/wasi_fs.rs`, which runs a real `wasm32-wasip1` fixture against a VFS.

- `run_module_with_mount(bytes, argv, stdin, WasiLimits, Option<Mount>)` runs a `_start` command module. Without a mount the guest has no files. With a mount, the VFS directory `Mount::root` is the guest's preopen (fd 3, named `.`).
- Guest paths are relative to the preopen. Absolute paths, `..`, and NUL bytes are refused with `ENOTCAPABLE` or `EINVAL`. Open handles are released when a run ends.
- Implemented preview1 calls: args and environ (empty), clock, random, `fd_read`/`fd_write`/`fd_seek`/`fd_tell`/`fd_close`, `fd_fdstat`/`fd_filestat`/`fd_filestat_set_size`, `fd_prestat*`, `path_open`, `path_filestat_get`, `path_create_directory`, `path_remove_directory`, `path_unlink_file`, `path_rename`, `fd_readdir`, `fd_sync`/`fd_datasync`, `sched_yield`, and `proc_exit`. `poll_oneoff`, `path_readlink`, and `path_symlink` return `ENOTSUP`. Modules that import anything else fail to instantiate.
- Stdio is in memory, capped per stream (`max_output_bytes`, default 1 MiB). CPU is limited by fuel (`fuel`, default 100M). Linear memory is capped by `StoreLimits` (`max_memory_bytes`, default 64 MiB).
- A guest exit code is returned as `exit_code`. A trap is exit 1 with a message on stderr. An invalid module is a host error.
- AOT: each module is compiled once per process and cached as a serialized artifact keyed by its BLAKE3 hash. Only this process writes that cache, so the `unsafe` deserialize is limited to bytes this process produced. Artifacts are not written to disk or to the VFS.
- Tool registry (`registry.rs`): the shell reads `/etc/tools.toml`. Each `[[tool]]` entry gives a name, a VFS path, an optional `hash` (BLAKE3 hex, checked before every run), and optional per-tool `fuel`, `max_memory_bytes`, and `max_output_bytes`. A malformed manifest fails closed.

Not supported: a persistent on-disk AOT cache (it would need integrity checks before loading), WASI preview2 components, sockets, and symlinks through the guest.

## Overview

- `wasi.rs`: engine, module cache, the preview1 host functions, and `execute`.
- `registry.rs`: `Registry` and `ToolSpec` parsing and hash checks.
- `fixtures/wasi_tool/`: the test program. Rebuild with `cargo build --release --target wasm32-wasip1` in that directory, then copy `target/wasm32-wasip1/release/wasi_tool.wasm` to `fixtures/wasi_tool.wasm`.
