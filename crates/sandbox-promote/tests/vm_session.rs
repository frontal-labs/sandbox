//! A Tier 1 session promoted into an emulated guest, then used only through the `Session`
//! trait: its commands and its files run in the guest. Ignored unless SANDBOX_VM_IMAGE names
//! a built guest image (docker/run-vm-tests.sh builds it).

use sandbox_promote::promote_to_vm;
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{ExecRequest, SessionConfig, TenantId, Tier};
use sandbox_vfs::OpenFlags;
use sandbox_vmm::{Accel, QemuSpec, QemuVm};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn spec(dir: &Path) -> QemuSpec {
    QemuSpec {
        qemu: PathBuf::from("qemu-system-x86_64"),
        kernel: dir.join("vmlinuz"),
        initrd: dir.join("initrd.gz"),
        memory_mib: 2048,
        accel: Accel::Tcg,
        console: dir.join("vm-session-console.log"),
    }
}

fn argv(words: &[&str]) -> ExecRequest {
    ExecRequest {
        argv: words.iter().map(|w| w.to_string()).collect(),
        ..ExecRequest::default()
    }
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn a_promoted_session_runs_and_stores_its_files_in_the_guest() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));

    // A Tier 1 session with a file in it.
    let session = IsolateBackend::new(SessionConfig {
        tenant_id: TenantId::new(),
        ..SessionConfig::default()
    })
    .await
    .unwrap();
    session
        .run_script("mkdir -p /work && echo before > /work/note.txt")
        .await
        .unwrap();

    let mut vm = QemuVm::start(&spec(&dir)).await.unwrap();
    vm.wait_for_agent(Duration::from_secs(180)).await.unwrap();
    let (promoted, report) = promote_to_vm(&session, vm).await.unwrap();
    assert!(report.files >= 1, "{report:?}");
    assert_eq!(promoted.tier(), Tier::Vm);

    // Commands run in the guest, against the promoted files.
    let out = promoted
        .exec(argv(&["cat", "/work/note.txt"]))
        .await
        .unwrap();
    assert_eq!(
        (out.exit_code, out.stdout.as_slice()),
        (0, b"before\n".as_slice())
    );

    // A write through the session's filesystem lands in the guest, where a command sees it.
    let flags = OpenFlags {
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = promoted
        .fs()
        .open(Path::new("/work/guest.txt"), flags)
        .await
        .unwrap();
    promoted
        .fs()
        .write(&handle, 0, b"written in the guest\n")
        .await
        .unwrap();
    promoted.fs().close(&handle);
    let out = promoted
        .exec(argv(&["cat", "/work/guest.txt"]))
        .await
        .unwrap();
    assert_eq!(out.stdout, b"written in the guest\n");

    // Files are read back, renamed, and removed through the same filesystem.
    let stat = promoted
        .fs()
        .stat(Path::new("/work/guest.txt"))
        .await
        .unwrap();
    assert!(stat.is_file && stat.size == 21, "{stat:?}");
    promoted
        .fs()
        .rename(Path::new("/work/guest.txt"), Path::new("/work/renamed.txt"))
        .await
        .unwrap();
    assert!(promoted
        .fs()
        .stat(Path::new("/work/guest.txt"))
        .await
        .is_err());
    promoted
        .fs()
        .rm(Path::new("/work/renamed.txt"), false)
        .await
        .unwrap();

    // The root is the guest's own: it changes when the guest's files change.
    let before = promoted.snapshot().await.unwrap();
    promoted
        .run_script("echo more >> /work/note.txt")
        .await
        .unwrap();
    let after = promoted.snapshot().await.unwrap();
    assert_ne!(before, after);
    let checkpoint = promoted.checkpoint().await.unwrap();
    assert_eq!(checkpoint.root, after);

    Box::new(promoted).destroy().await.unwrap();
}
