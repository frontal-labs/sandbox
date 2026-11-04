//! Exports a tree as an ext4 image and reads it back with e2fsprogs. Linux only, because the
//! image is made with mkfs.ext4, which the dev image has (docker/run-linux.sh runs this test).

#![cfg(target_os = "linux")]

use sandbox_cas::MemStore;
use sandbox_promote::export_ext4;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

async fn write(vfs: &Vfs, path: &str, data: &[u8]) {
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

#[tokio::test]
async fn a_tree_exports_to_an_ext4_image_that_checks_clean_and_holds_the_files() {
    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/work/deep"), 0o755).await.unwrap();
    write(&vfs, "/work/hello.txt", b"hello from the session\n").await;
    write(&vfs, "/work/deep/empty", b"").await;
    let big: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    write(&vfs, "/work/deep/big.bin", &big).await;

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
