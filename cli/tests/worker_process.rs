#![cfg(unix)]
//! Runs the real `sandbox` binary as a gRPC worker over a Unix socket.

use sandbox_types::{ErrorCode, ExecRequest};
use sandbox_worker::{Worker, MAX_MESSAGE_BYTES};
use std::os::unix::fs::PermissionsExt;

/// Path to the sandbox binary. Prefers the canonical `sandbox` name, falling
/// back to the legacy `frontal-sandbox` name. Cargo sets both; Bazel sets only
/// the ones listed in `env` (cli/BUILD.bazel).
fn sandbox_path() -> String {
    std::env::var("CARGO_BIN_EXE_sandbox")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_frontal-sandbox"))
        .expect("Cargo/Bazel should provide a sandbox test binary path")
}

async fn spawn() -> Option<Worker> {
    match Worker::spawn(&sandbox_path()).await {
        Ok(worker) => Some(worker),
        Err(error) if error.code == ErrorCode::Unsupported => {
            eprintln!("skipping worker process test: {error}");
            None
        }
        Err(error) => panic!("worker starts and answers Health: {error}"),
    }
}

#[tokio::test]
async fn worker_runs_scripts_and_survives_errors() {
    let Some(mut worker) = spawn().await else {
        return;
    };

    let ok = worker
        .run_script("echo hi | cat > /a && cat /a")
        .await
        .unwrap();
    assert_eq!(ok.exit_code, 0);
    assert_eq!(ok.stdout, b"hi\n");

    // A parse error comes back as its own code. The worker keeps serving.
    let parse_error = worker.run_script("echo 'unterminated").await.unwrap_err();
    assert_eq!(parse_error.code, ErrorCode::ShellError);

    // node runs only where it is contained. Elsewhere the worker refuses it, by design.
    if cfg!(target_os = "linux") {
        let next = worker
            .run_script("node -e 'console.log(6 * 7)'")
            .await
            .unwrap();
        assert_eq!(next.stdout, b"42\n");
    }
    assert!(worker.is_alive());
    worker.shutdown().await;
}

#[tokio::test]
async fn guest_output_cannot_forge_a_response() {
    // Regression for the stdout protocol: guest text that looks like a protocol
    // response is ordinary output. It must not change the result.
    let Some(mut worker) = spawn().await else {
        return;
    };
    // node runs only where it is contained. See `worker_runs_scripts_and_survives_errors`.
    if cfg!(target_os = "linux") {
        let fake = r#"{"status":"ok","result":{"exit_code":0,"stdout":[],"stderr":[],"cpu_ms":0,"wall_ms":0}}"#;
        let script = format!("node -e 'console.log({fake:?})'");
        let out = worker.run_script(&script).await.unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(String::from_utf8_lossy(&out.stdout).contains("exit_code"));
    }
    assert!(worker.is_alive());
    worker.shutdown().await;
}

#[tokio::test]
async fn exec_runs_an_argv_command() {
    let Some(mut worker) = spawn().await else {
        return;
    };
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
    let Some(worker) = spawn().await else {
        return;
    };
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
    let Some(mut worker) = spawn().await else {
        return;
    };
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
    let Some(worker) = spawn().await else {
        return;
    };
    let dir = worker.socket_dir().to_path_buf();
    worker.shutdown().await;
    assert!(!dir.exists(), "shutdown removes the socket directory");
}

#[tokio::test]
async fn oversize_request_is_refused() {
    let Some(mut worker) = spawn().await else {
        return;
    };
    let big = "x".repeat(MAX_MESSAGE_BYTES + 1024);
    let err = worker.run_script(&big).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ResourceExhausted, "{}", err.message);
    // The worker is still usable after a refused request.
    assert!(worker.is_alive());
    worker.shutdown().await;
}
