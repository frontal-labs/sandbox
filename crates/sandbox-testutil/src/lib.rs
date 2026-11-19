//! Shared helpers for the sandbox test suites: the host-canary probe and a small
//! in-memory VFS factory. These were copy-pasted across the WASM, shell, and runtime
//! escape tests; they live here so the three suites stay in sync.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::path::Path;
use std::sync::Arc;

/// The default canary payload. Suites that need a distinct marker (so a test that
/// searches the whole filesystem cannot trip over a sibling suite's file) may pass
/// their own string to [HostCanary::with_content].
pub const CANARY: &str = "HOST-CANARY-5c0e7a13";

/// A directory on the real host holding a canary file. A hostile guest that reaches
/// the host filesystem reads, modifies, or adds files here; [HostCanary::assert_untouched]
/// fails the test if either happened. The directory is removed on drop.
pub struct HostCanary {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
    content: &'static str,
}

impl HostCanary {
    /// Creates `temp/escape-{tag}-{pid}/canary.txt` containing [CANARY].
    pub fn new(tag: &str) -> Self {
        Self::with_content(tag, CANARY)
    }

    /// Like [Self::new] but with a suite-specific canary string.
    pub fn with_content(tag: &str, content: &'static str) -> Self {
        let dir = std::env::temp_dir().join(format!("escape-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("canary.txt");
        std::fs::write(&file, content).unwrap();
        Self { dir, file, content }
    }

    /// The host path of the canary file.
    pub fn path(&self) -> &str {
        self.file.to_str().unwrap()
    }

    /// The host directory containing the canary.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// The host directory has exactly the canary file, unchanged.
    pub fn assert_untouched(&self) {
        let names: Vec<_> = std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["canary.txt".to_string()],
            "host directory changed"
        );
        assert_eq!(std::fs::read_to_string(&self.file).unwrap(), self.content);
    }
}

impl Drop for HostCanary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fresh in-memory VFS.
pub fn new_vfs() -> Arc<Vfs> {
    Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())))
}

/// Writes `data` to `path` in `vfs`, creating the file.
pub async fn put(vfs: &Vfs, path: &str, data: &[u8]) {
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new(path), flags).await.unwrap();
    vfs.write(&handle, 0, data).await.unwrap();
    vfs.close(&handle);
}
