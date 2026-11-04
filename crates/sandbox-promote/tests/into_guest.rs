//! Promotes a filesystem into a guest booted under QEMU, then reads it back and runs a
//! command there. Ignored unless SANDBOX_VM_IMAGE names a built guest image (see
//! docker/run-vm-tests.sh).

use sandbox_cas::MemStore;
use sandbox_promote::{promote_tree, CHUNK_BYTES};
use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::guest::{ListDirRequest, ReadFileRequest};
use sandbox_proto::v1::types::ExecRequest;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_vmm::{Accel, QemuSpec, QemuVm};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn a_promoted_tree_is_readable_and_runnable_in_the_guest() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let mut vm = QemuVm::start(&QemuSpec {
        qemu: PathBuf::from("qemu-system-x86_64"),
        kernel: dir.join("vmlinuz"),
        initrd: dir.join("initrd.gz"),
        memory_mib: 2048,
        accel: Accel::from_env(),
        console: dir.join("promote-test-console.log"),
    })
    .await
    .unwrap();
    vm.wait_for_agent(Duration::from_secs(180)).await.unwrap();
    let mut guest = GuestServiceClient::connect(format!("http://{}", vm.agent()))
        .await
        .unwrap();

    // A tree with nesting, a file that spans several chunks, and an empty file.
    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/work/sub"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/work/sub/deep"), 0o755).await.unwrap();
    write(&vfs, "/work/a.txt", b"hello from the session\n").await;
    write(&vfs, "/work/empty", b"").await;
    let big: Vec<u8> = (0..3 * CHUNK_BYTES + 12_345)
        .map(|i| (i % 251) as u8)
        .collect();
    write(&vfs, "/work/sub/deep/big.bin", &big).await;

    let report = match promote_tree(&vfs, &mut guest).await {
        Ok(report) => report,
        Err(error) => {
            let console =
                std::fs::read_to_string(dir.join("promote-test-console.log")).unwrap_or_default();
            panic!("promotion failed: {error}\n--- guest console ---\n{console}");
        }
    };
    assert_eq!(report.directories, 3, "{report:?}");
    assert_eq!(report.files, 3, "{report:?}");
    assert_eq!(
        report.bytes,
        (23 + 3 * CHUNK_BYTES + 12_345) as u64,
        "{report:?}"
    );

    // The guest holds the same bytes, read back in chunks.
    let mut read = Vec::new();
    let mut offset = 0u64;
    let mut eof = false;
    while !eof {
        let chunk = guest
            .read_file(ReadFileRequest {
                path: "/work/sub/deep/big.bin".into(),
                offset,
                max_bytes: CHUNK_BYTES as u32,
            })
            .await
            .unwrap()
            .into_inner();
        offset += chunk.data.len() as u64;
        read.extend_from_slice(&chunk.data);
        eof = chunk.eof;
    }
    assert_eq!(read, big);

    let listing = guest
        .list_dir(ListDirRequest {
            path: "/work".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"a.txt") && names.contains(&"empty") && names.contains(&"sub"),
        "{names:?}"
    );

    let exec = guest
        .exec(ExecRequest {
            argv: vec!["cat".into(), "/work/a.txt".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (exec.exit_code, exec.stdout.as_slice()),
        (0, b"hello from the session\n".as_slice())
    );

    vm.stop().await.unwrap();
}
