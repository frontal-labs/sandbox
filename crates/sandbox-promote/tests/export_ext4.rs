//! Exports a tree as an ext4 image, reads it back with e2fsprogs, and imports it into a fresh
//! VFS. Linux only, because the image is made with mkfs.ext4, which the dev image has
//! (docker/run-linux.sh runs this test).

#![cfg(target_os = "linux")]

use sandbox_cas::MemStore;
use sandbox_promote::{export_ext4, import_from_dir};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

async fn put(vfs: &Vfs, path: &str, data: &[u8]) {
    let flags = OpenFlags {
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new(path), flags).await.unwrap();
    vfs.write(&handle, 0, data).await.unwrap();
    vfs.close(&handle);
}

/// Recursively collects every file in `vfs` under `/` into a `path -> bytes` map, for
/// comparing the exported and imported trees.
async fn snapshot(vfs: &Vfs) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec!["/".to_string()];
    while let Some(dir) = pending.pop() {
        let mut entries = vfs.readdir(Path::new(&dir)).await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            let path = if dir == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_dir {
                pending.push(path);
            } else if entry.is_file {
                let data = read_bytes(vfs, &path).await;
                out.insert(path, data);
            }
        }
    }
    out
}

async fn read_bytes(vfs: &Vfs, path: &str) -> Vec<u8> {
    let handle = vfs
        .open(Path::new(path), OpenFlags::default())
        .await
        .unwrap();
    let mut out = Vec::new();
    let mut done = false;
    while !done {
        let chunk = vfs
            .read(&handle, out.len() as u64, 1024 * 1024)
            .await
            .unwrap();
        done = chunk.len() < 1024 * 1024;
        out.extend_from_slice(&chunk);
    }
    vfs.close(&handle);
    out
}

#[tokio::test]
async fn a_tree_exports_to_an_ext4_image_that_checks_clean_and_holds_the_files() {
    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/work/deep"), 0o755).await.unwrap();
    put(&vfs, "/work/hello.txt", b"hello from the session\n").await;
    put(&vfs, "/work/deep/empty", b"").await;
    let big: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    put(&vfs, "/work/deep/big.bin", &big).await;

    let dir = std::env::temp_dir().join(format!("export-ext4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("root.ext4");
    let report = export_ext4(&vfs, &image, &dir.join("scratch"))
        .await
        .unwrap();
    assert_eq!(report.files, 3, "{report:?}");
    assert_eq!(report.directories, 2, "{report:?}");

    let check = Command::new("e2fsck")
        .arg("-fn")
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "e2fsck: {}",
        String::from_utf8_lossy(&check.stdout)
    );

    let cat = Command::new("debugfs")
        .args(["-R", "cat /work/hello.txt"])
        .arg(&image)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&cat.stdout),
        "hello from the session\n"
    );

    let size = Command::new("debugfs")
        .args(["-R", "stat /work/deep/big.bin"])
        .arg(&image)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&size.stdout).contains("Size: 3145728"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_exported_image_imports_back_into_a_fresh_vfs_with_the_same_tree() {
    let source = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    source.mkdir(Path::new("/work"), 0o755).await.unwrap();
    source.mkdir(Path::new("/work/deep"), 0o755).await.unwrap();
    put(&source, "/work/hello.txt", b"round trip\n").await;
    put(&source, "/work/deep/empty", b"").await;
    let payload: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 197) as u8).collect();
    put(&source, "/work/deep/data.bin", &payload).await;
    put(&source, "/top.txt", b"top level\n").await;

    let dir = std::env::temp_dir().join(format!("import-ext4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("root.ext4");
    let exported = export_ext4(&source, &image, &dir.join("scratch"))
        .await
        .unwrap();

    let destination = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    let imported = import_from_dir(&destination, &image, &dir.join("scratch-in"))
        .await
        .unwrap();

    assert_eq!(imported.files, exported.files, "{imported:?}");
    assert_eq!(imported.directories, exported.directories, "{imported:?}");
    assert_eq!(imported.bytes, exported.bytes, "{imported:?}");

    let before = snapshot(&source).await;
    let after = snapshot(&destination).await;
    assert_eq!(
        before, after,
        "imported tree differs from the exported tree"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
