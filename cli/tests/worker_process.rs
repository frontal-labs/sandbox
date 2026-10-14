//! Runs the real `sandbox` binary as a gRPC worker over a Unix socket.

use sandbox_types::{ErrorCode, ExecRequest};
use sandbox_worker::{Worker, MAX_MESSAGE_BYTES};
use std::os::unix::fs::PermissionsExt;

async fn spawn() -> Worker {
    Worker::spawn(env!("CARGO_BIN_EXE_sandbox"))
        .await
        .expect("worker starts and answers Health")
}

#[tokio::test]
async fn worker_runs_scripts_and_survives_errors() {
    let mut worker = spawn().await;

    let ok = worker
        .run_script("echo hi | cat > /a && cat /a")
        .await
        .unwrap();
    assert_eq!(ok.exit_code, 0);
    assert_eq!(ok.stdout, b"hi\n");

    // A parse error comes back as its own code. The worker keeps serving.
    let parse_error = worker.run_script("echo 'unterminated").await.unwrap_err();
    assert_eq!(parse_error.code, ErrorCode::ShellError);

    let next = worker
        .run_script("node -e 'console.log(6 * 7)'")
        .await
        .unwrap();
    assert_eq!(next.stdout, b"42\n");
    assert!(worker.is_alive());
    worker.shutdown().await;
}

#[tokio::test]
async fn guest_output_cannot_forge_a_response() {
    // Regression for the stdout protocol: guest text that looks like a protocol
    // response is ordinary output. It must not change the result.
    let mut worker = spawn().await;
    let fake = r#"{"status":"ok","result":{"exit_code":0,"stdout":[],"stderr":[],"cpu_ms":0,"wall_ms":0}}"#;
    let script = format!("node -e 'console.log({fake:?})'");
    let out = worker.run_script(&script).await.unwrap();
    assert_eq!(out.exit_code, 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("exit_code"));
    assert!(worker.is_alive());
    worker.shutdown().await;
}

#[tokio::test]
async fn exec_runs_an_argv_command() {
    let mut worker = spawn().await;
    let req = ExecRequest {
        argv: vec!["echo".to_string(), "via-exec".to_string()],
        ..ExecRequest::default()
    };
    let out = worker.exec(req).await.unwrap();
    assert_eq!(out.stdout, b"via-exec\n");
    worker.shutdown().await;
}

#[tokio::test]
async fn socket_directory_is_private_and_removed_on_drop() {
    let worker = spawn().await;
    let dir = worker.socket_dir().to_path_buf();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "socket directory must not be readable by others"
    );
    assert!(dir.join("worker.sock").exists());
    drop(worker);
    assert!(!dir.exists(), "socket directory removed on drop");
}

#[tokio::test]
async fn a_killed_worker_reports_unavailable() {
    let mut worker = spawn().await;
    let pid = worker.pid().expect("child is running");
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("kill runs");
    assert!(status.success());
    // Give the kernel a moment to close the socket.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let err = worker.run_script("echo after-kill").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable, "{}", err.message);
    assert!(!worker.is_alive());
}

#[tokio::test]
async fn shut_down_worker_removes_its_socket_directory() {
    let worker = spawn().await;
    let dir = worker.socket_dir().to_path_buf();
    worker.shutdown().await;
    assert!(!dir.exists(), "shutdown removes the socket directory");
}

#[tokio::test]
async fn oversize_request_is_refused() {
    let mut worker = spawn().await;
    let big = "x".repeat(MAX_MESSAGE_BYTES + 1024);
    let err = worker.run_script(&big).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ResourceExhausted, "{}", err.message);
    // The worker is still usable after a refused request.
    assert!(worker.is_alive());
    worker.shutdown().await;
}
