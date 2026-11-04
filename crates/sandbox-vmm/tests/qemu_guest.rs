//! Boots the guest image under QEMU and talks to its agent. Ignored unless the image is
//! given (docker/run-vm-tests.sh builds it and sets the variables), because it needs QEMU
//! and a kernel.

use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_vmm::{Accel, QemuSpec, QemuVm};
use std::path::PathBuf;
use std::time::Duration;

fn spec_from_env() -> QemuSpec {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    QemuSpec {
        qemu: PathBuf::from(std::env::var("SANDBOX_QEMU").unwrap_or("qemu-system-x86_64".into())),
        kernel: dir.join("vmlinuz"),
        initrd: dir.join("initrd.gz"),
        memory_mib: 512,
        accel: Accel::Tcg,
        console: dir.join("qemu-test-console.log"),
    }
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn the_guest_agent_answers_after_a_qemu_boot_and_stops_with_the_vm() {
    let mut vm = QemuVm::start(&spec_from_env()).await.unwrap();
    vm.wait_for_agent(Duration::from_secs(180)).await.unwrap();

    let mut client = GuestServiceClient::connect(format!("http://{}", vm.agent()))
        .await
        .unwrap();
    let ping = client.ping(()).await.unwrap().into_inner();
    assert!(!ping.agent_version.is_empty());

    let addr = vm.agent();
    vm.stop().await.unwrap();
    // Once stopped, the agent's port no longer accepts connections.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(addr))
            .await
            .map_or(true, |connected| connected.is_err()),
        "the agent still answers at {addr} after stop"
    );
}
