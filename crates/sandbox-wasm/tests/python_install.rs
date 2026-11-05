//! Installing CPython into a session: the module, the stdlib, and the manifest entry land in
//! the session's filesystem, a second install keeps one entry and the other tools, and the
//! entry parses. The bytes here are stand-ins; the real artifacts are checked by the ignored
//! conformance tests.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_wasm::{install_python, PythonArtifacts, Registry};
use std::path::Path;
use std::sync::Arc;

async fn read(vfs: &Vfs, path: &str) -> Vec<u8> {
    let stat = vfs.stat(Path::new(path)).await.unwrap();
    let handle = vfs
        .open(Path::new(path), OpenFlags::default())
        .await
        .unwrap();
    let bytes = vfs
        .read(&handle, 0, stat.size as usize)
        .await
        .unwrap()
        .to_vec();
    vfs.close(&handle);
    bytes
}

#[tokio::test]
async fn an_install_writes_the_artifacts_and_keeps_one_python_entry() {
    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    // An existing manifest with another tool, which the install must keep.
    vfs.mkdir(Path::new("/etc"), 0o755).await.unwrap();
    let flags = OpenFlags {
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new("/etc/tools.toml"), flags).await.unwrap();
    let other = "[[tool]]\nname = \"jq\"\npath = \"/opt/jq.wasm\"\n";
    vfs.write(&handle, 0, other.as_bytes()).await.unwrap();
    vfs.close(&handle);

    let artifacts = PythonArtifacts {
        module: vec![7u8; 3 * 1024 * 1024 + 5],
        stdlib: vec![9u8; 1024 * 1024 + 1],
    };
    install_python(&vfs, &artifacts).await.unwrap();
    install_python(&vfs, &artifacts).await.unwrap();

    assert_eq!(read(&vfs, "/opt/python.wasm").await, artifacts.module);
    assert_eq!(read(&vfs, "/usr/lib/python313.zip").await, artifacts.stdlib);
    let manifest = String::from_utf8(read(&vfs, "/etc/tools.toml").await).unwrap();
    let registry = Registry::parse(&manifest).unwrap();
    assert!(registry.find("jq").is_some(), "{manifest}");
    assert_eq!(
        manifest.matches("name = \"python\"").count(),
        1,
        "{manifest}"
    );
    let python = registry.find("python").unwrap();
    assert_eq!(python.argv0.as_deref(), Some("/usr/bin/python"));
    assert_eq!(python.preopen, vec!["/usr".to_string()]);
}
