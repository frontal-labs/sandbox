//! A guest saved with `savevm` resumes in the state it was saved in: a directory the agent made
//! before the save is there after the resume, and the agent answers at the new address. Ignored
//! unless the image is given, and it needs `qemu-img` to make the state disk (docker/run-vm-tests.sh).

use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::guest::{MkDirRequest, StatRequest};
use sandbox_vmm::{Accel, QemuSpec, QemuVm, Snapshotting};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn spec_from_env() -> QemuSpec {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    QemuSpec {
        qemu: PathBuf::from(std::env::var("SANDBOX_QEMU").unwrap_or("qemu-system-x86_64".into())),
        kernel: dir.join("vmlinuz"),
        initrd: dir.join("initrd.gz"),
        memory_mib: 2048,
        accel: Accel::from_env(),
        console: dir.join("snapshot-test-console.log"),
    }
}

fn make_disk(path: &Path) {
    let status = Command::new("qemu-img")
        .args(["create", "-f", "qcow2"])
        .arg(path)
        .arg("64M")
        .status()
        .expect("qemu-img runs");
    assert!(status.success(), "qemu-img could not create the state disk");
}

async fn agent(vm: &QemuVm) -> GuestServiceClient<tonic::transport::Channel> {
    GuestServiceClient::connect(format!("http://{}", vm.agent()))
        .await
        .expect("the agent accepts connections")
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE and qemu-img; see docker/run-vm-tests.sh"]
async fn a_saved_guest_resumes_with_the_state_it_was_saved_in() {
    let work = std::env::temp_dir().join(format!("snapshot-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let disk = work.join("state.qcow2");
    make_disk(&disk);
    let monitor = work.join("monitor.sock");

    // Boot, make a directory in the guest, and save.
    let saving = Snapshotting {
        disk: disk.clone(),
        monitor: monitor.clone(),
        resume: None,
    };
    let mut vm = QemuVm::start_with(&spec_from_env(), Some(&saving))
        .await
        .unwrap();
    vm.wait_for_agent(Duration::from_secs(300)).await.unwrap();
    agent(&vm)
        .await
        .mk_dir(MkDirRequest {
            path: "/before-save".into(),
        })
        .await
        .unwrap();
    vm.save_state("resume-point").await.unwrap();
    vm.stop().await.unwrap();

    // Resume from the save: no boot, and the directory the agent made is still there.
    let resuming = Snapshotting {
        disk: disk.clone(),
        monitor: monitor.clone(),
        resume: Some("resume-point".into()),
    };
    let started = std::time::Instant::now();
    let mut resumed = QemuVm::start_with(&spec_from_env(), Some(&resuming))
        .await
        .unwrap();
    resumed
        .wait_for_agent(Duration::from_secs(300))
        .await
        .unwrap();
    eprintln!("RESUME_MS {}", started.elapsed().as_millis());
    let stat = agent(&resumed)
        .await
        .stat(StatRequest {
            path: "/before-save".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        stat.is_dir,
        "the directory made before the save is missing after the resume"
    );
    resumed.stop().await.unwrap();
    let _ = std::fs::remove_dir_all(&work);
}
