#![no_main]

use libfuzzer_sys::fuzz_target;
use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::path::Path;
use std::sync::{Arc, OnceLock};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("fuzz runtime")
    })
}

/// A path that climbs above its parent. Every operation must refuse it, so no
/// path can name anything outside the store's root (PLAN §4.3).
fn escapes(path: &str) -> bool {
    path.split('/').any(|part| part == "..")
}

// Each input line is `op path [path]`. The operations run against a fresh
// in-memory filesystem. The target asserts the path-safety rule and that
// committing never fails or panics.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    runtime().block_on(async {
        let mut vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
        for line in text.lines().take(64) {
            let mut fields = line.splitn(3, ' ');
            let (Some(op), Some(first)) = (fields.next(), fields.next()) else {
                continue;
            };
            let second = fields.next();
            let path = Path::new(first);
            let result = match (op, second) {
                ("mkdir", _) => vfs.mkdir(path, 0o755).await,
                ("rm", _) => vfs.rm(path, true).await,
                ("stat", _) => vfs.stat(path).await.map(|_| ()),
                ("readdir", _) => vfs.readdir(path).await.map(|_| ()),
                ("truncate", _) => vfs.truncate(path, 0).await,
                ("create", _) => {
                    let flags = OpenFlags {
                        read: false,
                        write: true,
                        create: true,
                        ..OpenFlags::default()
                    };
                    vfs.open(path, flags).await.map(|_| ())
                }
                ("rename", Some(to)) => vfs.rename(path, Path::new(to)).await,
                ("symlink", Some(link)) => vfs.symlink(path, Path::new(link)).await,
                ("readlink", _) => vfs.readlink(path).await.map(|_| ()),
                _ => continue,
            };
            // The paths this operation uses. A symlink's target is stored as
            // text, so it may climb; the link itself is a path in the tree, and
            // must not.
            let named: Vec<&str> = match (op, second) {
                ("rename", Some(to)) => vec![first, to],
                ("symlink", Some(link)) => vec![link],
                _ => vec![first],
            };
            if named.iter().any(|p| escapes(p)) {
                assert!(result.is_err(), "{op} accepted a path that climbs above root: {named:?}");
            }
        }
        let _ = vfs.commit().await;
    });
});
