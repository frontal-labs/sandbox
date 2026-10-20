#![no_main]

use libfuzzer_sys::fuzz_target;
use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits};
use std::sync::{Arc, OnceLock};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fuzz runtime")
    })
}

/// A path that leaves the preopened directory. The host must refuse every one.
fn escapes(path: &[u8]) -> bool {
    path.first() == Some(&b'/')
        || path.contains(&0)
        || path.split(|&b| b == b'/').any(|part| part == b"..")
}

/// A WAT module that calls one WASI path function with `path` in its memory and
/// exits with the errno the host returned. The path is written as hex escapes, so
/// any bytes are valid.
fn module(op: u8, path: &[u8], second: &[u8]) -> String {
    let escaped = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("\\{b:02x}")).collect() };
    let (call, extra_data) = match op % 4 {
        // path_open(dirfd, lookupflags, path, path_len, oflags, rights, inheriting, fdflags, out)
        0 => (
            "(call $path_open (i32.const 3) (i32.const 0) (i32.const 64) (i32.const PLEN) \
             (i32.const 0) (i64.const -1) (i64.const -1) (i32.const 0) (i32.const 1024))",
            String::new(),
        ),
        // path_create_directory(fd, path, path_len)
        1 => (
            "(call $path_create_directory (i32.const 3) (i32.const 64) (i32.const PLEN))",
            String::new(),
        ),
        // path_unlink_file(fd, path, path_len)
        2 => (
            "(call $path_unlink_file (i32.const 3) (i32.const 64) (i32.const PLEN))",
            String::new(),
        ),
        // path_rename(fd, old, old_len, new_fd, new, new_len)
        _ => (
            "(call $path_rename (i32.const 3) (i32.const 64) (i32.const PLEN) \
             (i32.const 3) (i32.const 512) (i32.const SLEN))",
            format!("(data (i32.const 512) \"{}\")", escaped(second)),
        ),
    };
    let call = call
        .replace("PLEN", &path.len().to_string())
        .replace("SLEN", &second.len().to_string());
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (import "wasi_snapshot_preview1" "path_open" (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_create_directory" (func $path_create_directory (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_unlink_file" (func $path_unlink_file (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_rename" (func $path_rename (param i32 i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 64) "{}")
  {}
  (func (export "_start")
    (call $proc_exit {})))"#,
        escaped(path),
        extra_data,
        call
    )
}

// The guest's path functions are driven with fuzzed paths. A path that escapes
// the preopened directory must never succeed (errno 0). Other paths may do
// anything the VFS allows, but must not panic or leave the sandbox.
fuzz_target!(|data: &[u8]| {
    if data.len() < 2 || data.len() > 512 {
        return;
    }
    let op = data[0];
    let split = usize::from(data[1]).min(data.len() - 2);
    let rest = &data[2..];
    let (path, second) = rest.split_at(split.min(rest.len()));
    let wat = module(op, path, second);
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mount = Mount {
        vfs,
        root: "/".to_string(),
    };
    let outcome = runtime().block_on(run_module_with_mount(
        wat.into_bytes(),
        vec!["fuzz".to_string()],
        Vec::new(),
        WasiLimits::default(),
        Some(mount),
    ));
    let Ok(outcome) = outcome else {
        return;
    };
    let escaping = escapes(path) || (op % 4 == 3 && escapes(second));
    if escaping {
        assert_ne!(
            outcome.exit_code, 0,
            "a path that leaves the mount succeeded: {path:?} / {second:?} (op {})",
            op % 4
        );
    }
});
