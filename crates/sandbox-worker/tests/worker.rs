//! Integration tests for the worker process handle.

use sandbox_proto::v1::worker::worker_service_client::WorkerServiceClient;
use sandbox_types::{ErrorCode, ExecRequest};
use sandbox_worker::isolation::Isolation;
use sandbox_worker::worker::MAX_MESSAGE_BYTES;
use sandbox_worker::Worker;
use tokio::process::Command;
use tonic::transport::Endpoint;

#[tokio::test]
async fn a_child_that_exits_at_once_is_unavailable_not_a_panic() {
    // `true` ignores its arguments and exits, so the worker never becomes ready.
    // It is started unisolated, so this checks readiness, not the isolation setup.
    let err = match Worker::spawn_with("true", Isolation::none()).await {
        Ok(_) => panic!("a non-worker process must not become ready"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::Unavailable, "{}", err.message);
}

#[test]
fn oversize_scripts_are_refused_before_sending() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // A channel that never connects: the size check must fail first.
        let endpoint = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let mut worker = Worker {
            child: Command::new("sleep").arg("30").spawn().unwrap(),
            client: WorkerServiceClient::new(endpoint),
            dir: std::env::temp_dir(),
            #[cfg(target_os = "linux")]
            cgroup: None,
            isolation: Isolation::none(),
        };
        let big = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let err = worker.run_script(&big).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ResourceExhausted);
        // Exec is checked the same way: stdin alone can push the message over the limit.
        let req = ExecRequest {
            argv: vec!["cat".into()],
            env: Default::default(),
            cwd: None,
            stdin: Some(vec![0u8; MAX_MESSAGE_BYTES]),
            timeout_ms: None,
            stream: false,
            output_limit: None,
        };
        let err = worker.exec(req).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ResourceExhausted);
        let _ = worker.child.kill().await;
        // Do not run Drop: it would remove the shared temp directory.
        std::mem::forget(worker);
    });
}
