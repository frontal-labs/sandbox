//! The guest protocol, served by the agent inside an emulated Linux VM. The VM is
//! booted by `docker/run-emulated-vm.sh` under QEMU's software emulator (TCG), because
//! this environment has no KVM. The test is ignored unless the script sets
//! `SANDBOX_GUEST_ADDR`, so `cargo test` never needs the VM.
//!
//! What it shows: the agent runs under a booted Linux kernel and answers the protocol
//! over TCP. It does not show that commands run in that kernel. `Exec` runs the
//! in-process shell, as on every other tier.

use sandbox_guest::guest::guest_service_client::GuestServiceClient;
use sandbox_guest::guest::{ListDirRequest, ReadFileRequest, WriteFileRequest};
use sandbox_proto::v1::types::ExecRequest;
use std::time::{Duration, Instant};
use tonic::transport::Channel;

/// Connects, retrying while the guest boots. Gives up after two minutes.
async fn connect(addr: &str) -> GuestServiceClient<Channel> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match GuestServiceClient::connect(addr.to_string()).await {
            Ok(client) => return client,
            Err(error) if Instant::now() < deadline => {
                eprintln!("waiting for the guest agent: {error}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => panic!("the guest agent never answered at {addr}: {error}"),
        }
    }
}

#[tokio::test]
#[ignore = "needs the emulated VM from docker/run-emulated-vm.sh"]
async fn the_agent_serves_the_guest_protocol_from_a_booted_kernel() {
    let addr = std::env::var("SANDBOX_GUEST_ADDR")
        .expect("set SANDBOX_GUEST_ADDR, or run docker/run-emulated-vm.sh");
    let mut client = connect(&addr).await;

    let ping = client.ping(()).await.unwrap().into_inner();
    assert!(
        !ping.agent_version.is_empty(),
        "the agent reports its version"
    );

    client
        .write_file(WriteFileRequest {
            path: "/hello.txt".to_string(),
            data: b"from the host".to_vec(),
        })
        .await
        .unwrap();
    let read = client
        .read_file(ReadFileRequest {
            path: "/hello.txt".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.data, b"from the host");

    let listing = client
        .list_dir(ListDirRequest {
            path: "/".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "hello.txt"),
        "{listing:?}"
    );

    let exec = client
        .exec(ExecRequest {
            argv: vec!["echo".to_string(), "hi".to_string()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (exec.exit_code, exec.stdout.as_slice()),
        (0, b"hi\n".as_slice())
    );

    client
        .sync(sandbox_guest::guest::SyncRequest::default())
        .await
        .unwrap();
}
