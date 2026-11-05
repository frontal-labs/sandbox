//! CPython, installed into a session, runs in a promoted Tier 2 guest: the promoted files include
//! the module, the stdlib, and the manifest entry, so the guest's shell finds `python`. Ignored
//! unless SANDBOX_VM_IMAGE and SANDBOX_PYTHON_DIR name the guest image and the artifacts.

use sandbox_cas::MemStore;
use sandbox_promote::promote_tree;
use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::types::ExecRequest;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use sandbox_vmm::{Accel, QemuSpec, QemuVm};
use sandbox_wasm::{install_python, PythonArtifacts};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE and SANDBOX_PYTHON_DIR; see docker/run-vm-tests.sh"]
async fn cpython_runs_in_a_promoted_guest() {
    let image = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let python =
        PathBuf::from(std::env::var("SANDBOX_PYTHON_DIR").expect("set SANDBOX_PYTHON_DIR"));
    let artifacts = PythonArtifacts::load(&python).unwrap();

    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    install_python(&vfs, &artifacts).await.unwrap();

    let mut vm = QemuVm::start(&QemuSpec {
        qemu: PathBuf::from("qemu-system-x86_64"),
        kernel: image.join("vmlinuz"),
        initrd: image.join("initrd.gz"),
        memory_mib: 2048,
        accel: Accel::from_env(),
        console: image.join("python-guest-console.log"),
    })
    .await
    .unwrap();
    vm.wait_for_agent(Duration::from_secs(300)).await.unwrap();
    let mut guest = GuestServiceClient::connect(format!("http://{}", vm.agent()))
        .await
        .unwrap();
    promote_tree(&vfs, &mut guest).await.unwrap();

    let out = guest
        .exec(ExecRequest {
            argv: vec!["python".into(), "-c".into(), "print(6 * 7)".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (out.exit_code, out.stdout.as_slice()),
        (0, b"42\n".as_slice()),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    vm.stop().await.unwrap();
}
