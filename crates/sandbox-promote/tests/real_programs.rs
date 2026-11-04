//! Programs run in the guest as real processes, not in its shell. The guest's kernel
//! answers `uname`, a promoted file is read by `sed`, and changes a program makes to the
//! tree (a new file, a removed one) come back to the session. Ignored unless SANDBOX_VM_IMAGE
//! names a built guest image (docker/run-vm-tests.sh builds it).

use sandbox_cas::MemStore;
use sandbox_promote::promote_tree;
use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::guest::ReadFileRequest;
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

fn request(argv: &[&str], stdin: &[u8]) -> ExecRequest {
    ExecRequest {
        argv: argv.iter().map(|a| a.to_string()).collect(),
        stdin: Some(stdin.to_vec()),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn programs_run_in_the_guest_and_their_changes_come_back() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let mut vm = QemuVm::start(&QemuSpec {
        qemu: PathBuf::from("qemu-system-x86_64"),
        kernel: dir.join("vmlinuz"),
        initrd: dir.join("initrd.gz"),
        memory_mib: 2048,
        accel: Accel::Tcg,
        console: dir.join("real-programs-console.log"),
    })
    .await
    .unwrap();
    vm.wait_for_agent(Duration::from_secs(180)).await.unwrap();
    let mut guest = GuestServiceClient::connect(format!("http://{}", vm.agent()))
        .await
        .unwrap();

    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    write(&vfs, "/work/a.txt", b"hello\n").await;
    promote_tree(&vfs, &mut guest).await.unwrap();

    // The kernel that answers is the guest's, not the host's.
    let uname = guest
        .exec(request(&["uname", "-r"], b""))
        .await
        .unwrap()
        .into_inner();
    let release = String::from_utf8_lossy(&uname.stdout).trim().to_string();
    assert_eq!(uname.exit_code, 0, "{uname:?}");
    assert!(
        release.starts_with("6."),
        "the guest kernel is 6.x: {release:?}"
    );

    // A program reads a promoted file.
    let sed = guest
        .exec(request(&["sed", "s/hello/bye/", "/work/a.txt"], b""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (sed.exit_code, sed.stdout.as_slice()),
        (0, b"bye\n".as_slice()),
        "{sed:?}"
    );

    // A program writes a new file, and the session sees it.
    let tee = guest
        .exec(request(&["tee", "/work/new.txt"], b"written\n"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(tee.exit_code, 0, "{tee:?}");
    let read = guest
        .read_file(ReadFileRequest {
            path: "/work/new.txt".into(),
            offset: 0,
            max_bytes: 0,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.data, b"written\n");

    // A program removes a file, and the session no longer has it.
    let unlink = guest
        .exec(request(&["unlink", "/work/a.txt"], b""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unlink.exit_code, 0, "{unlink:?}");
    let gone = guest
        .read_file(ReadFileRequest {
            path: "/work/a.txt".into(),
            offset: 0,
            max_bytes: 0,
        })
        .await;
    assert!(gone.is_err(), "the removed file is gone from the session");

    // A program from the image's /usr runs, and the copy's system mounts come off afterwards:
    // the guest still answers, from its own files.
    let ffmpeg = guest
        .exec(request(&["ffmpeg", "-version"], b""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ffmpeg.exit_code, 0, "{ffmpeg:?}");
    assert!(
        String::from_utf8_lossy(&ffmpeg.stdout).starts_with("ffmpeg version"),
        "{ffmpeg:?}"
    );
    let again = guest
        .exec(request(&["uname", "-r"], b""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.exit_code, 0, "{again:?}");
    assert!(
        String::from_utf8_lossy(&again.stdout)
            .trim()
            .starts_with("6."),
        "{again:?}"
    );

    // Chromium starts, then renders a page without a display, and its output is the DOM.
    let version = guest
        .exec(request(&["chromium", "--version"], b""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        version.exit_code,
        0,
        "chromium --version: exit {} stderr {:?}",
        version.exit_code,
        String::from_utf8_lossy(&version.stderr)
    );
    let chromium = guest
        .exec(request(
            &[
                "chromium",
                "--headless",
                "--no-sandbox",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--dump-dom",
                "data:text/html,<p>hello from the guest</p>",
            ],
            b"",
        ))
        .await
        .unwrap()
        .into_inner();
    if chromium.exit_code != 0 {
        let console =
            std::fs::read_to_string(dir.join("real-programs-console.log")).unwrap_or_default();
        let tail: Vec<&str> = console.lines().rev().take(40).collect();
        panic!(
            "chromium --dump-dom: exit {} stderr {:?} stdout {:?}\n--- guest console (last lines) ---\n{}",
            chromium.exit_code,
            String::from_utf8_lossy(&chromium.stderr),
            String::from_utf8_lossy(&chromium.stdout),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    assert!(
        String::from_utf8_lossy(&chromium.stdout).contains("<p>hello from the guest</p>"),
        "{}",
        String::from_utf8_lossy(&chromium.stdout)
    );

    vm.stop().await.unwrap();
}
