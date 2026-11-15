//! Integration tests for the isolate backend, moved out of src/backend.rs.

use sandbox_session::{
    changed_since, committed_root, quotas_for, restore, restore_checkpoint, IsolateBackend, Session,
};
use sandbox_shell::OutputObserver;
use sandbox_types::event::FileOp;
use sandbox_types::hash::ContentHash;
use sandbox_types::policy::ResourceLimits;
use sandbox_types::{ErrorCode, ExecRequest, SessionConfig, TenantId};
use sandbox_vfs::{OpenFlags, SessionFs};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

async fn new_backend() -> IsolateBackend {
    let config = SessionConfig {
        tenant_id: TenantId::new(),
        ..SessionConfig::default()
    };
    IsolateBackend::new(config).await.unwrap()
}

fn argv(words: &[&str]) -> ExecRequest {
    ExecRequest {
        argv: words.iter().map(|w| w.to_string()).collect(),
        ..ExecRequest::default()
    }
}

#[tokio::test]
async fn fs_writes_and_reads_back() {
    let backend = new_backend().await;
    let rw = OpenFlags {
        read: true,
        write: true,
        create: true,
        ..OpenFlags::default()
    };
    let fs = backend.fs();
    let h = fs.open(Path::new("/note.txt"), rw).await.unwrap();
    fs.write(&h, 0, b"kept").await.unwrap();
    let read = fs
        .open(Path::new("/note.txt"), OpenFlags::default())
        .await
        .unwrap();
    assert_eq!(fs.read(&read, 0, 16).await.unwrap().as_ref(), b"kept");
}

#[tokio::test]
async fn changed_since_lists_what_a_script_changed() {
    let backend = new_backend().await;
    backend.run_script("mkdir /work").await.unwrap();
    let before = committed_root(&backend).await.unwrap();
    backend.run_script("echo hi > /work/a.txt").await.unwrap();
    let changes = changed_since(&backend, before).await.unwrap();
    let listed: Vec<(&str, FileOp)> = changes.iter().map(|c| (c.path.as_str(), c.op)).collect();
    assert_eq!(listed, vec![("/work/a.txt", FileOp::Create)]);
    assert_eq!(changes[0].before, None);
    assert_eq!(changes[0].after, Some(ContentHash::new(b"hi\n")));
}

#[tokio::test]
async fn a_command_that_changes_nothing_lists_no_files() {
    let backend = new_backend().await;
    backend
        .run_script("mkdir /w && echo x > /w/f")
        .await
        .unwrap();
    let before = committed_root(&backend).await.unwrap();
    backend.run_script("cat /w/f").await.unwrap();
    assert!(changed_since(&backend, before).await.unwrap().is_empty());
}

#[test]
fn the_shorter_of_the_request_and_the_wall_timeout_applies() {
    assert_eq!(
        sandbox_session::effective_timeout(None, 300).0,
        Duration::from_secs(300)
    );
    assert_eq!(
        sandbox_session::effective_timeout(Some(50), 300).0,
        Duration::from_millis(50)
    );
    assert_eq!(
        sandbox_session::effective_timeout(Some(900_000), 300).0,
        Duration::from_secs(300)
    );
    assert!(sandbox_session::effective_timeout(Some(900_000), 300)
        .1
        .contains("wall timeout"));
    assert!(sandbox_session::effective_timeout(Some(50), 300)
        .1
        .contains("50 ms"));
}

#[test]
fn the_session_wall_timeout_is_the_sessions_own_limit_or_the_default() {
    let own = ResourceLimits {
        wall_timeout_secs: Some(10),
        ..ResourceLimits::default()
    };
    assert_eq!(sandbox_session::session_wall_secs(&own), 10);

    let unset = ResourceLimits {
        wall_timeout_secs: None,
        ..ResourceLimits::default()
    };
    assert_eq!(sandbox_session::session_wall_secs(&unset), 300);
}

#[test]
fn quotas_for_uses_max_total_size_fallback_to_default() {
    let explicit_total = ResourceLimits {
        max_total_size: Some(8192),
        memory_bytes: Some(4096),
        ..ResourceLimits::default()
    };
    assert_eq!(quotas_for(&explicit_total).total_bytes, 8192);

    let neither = ResourceLimits {
        max_total_size: None,
        memory_bytes: Some(4096),
        ..ResourceLimits::default()
    };
    assert_eq!(
        quotas_for(&neither).total_bytes,
        sandbox_vfs::Quotas::default().total_bytes
    );
}

#[tokio::test]
async fn exec_timeout_stops_a_long_command_with_a_timeout_error() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let backend = new_backend().await;
    let started = Instant::now();
    let mut req = argv(&["node", "-e", "let i=0; while(i<1000000000){i++}"]);
    req.timeout_ms = Some(50);
    let err = backend.exec(req).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn exec_streamed_reports_the_output_and_returns_it() {
    let backend = new_backend().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let observer: OutputObserver = Arc::new(move |chunk| sink.lock().unwrap().push(chunk));
    let out = backend
        .exec_streamed(argv(&["echo", "hi"]), observer)
        .await
        .unwrap();
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, b"hi\n");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].fd, 1);
    assert_eq!(seen[0].data, b"hi\n");
}

#[tokio::test]
async fn exec_refuses_a_streaming_request_so_it_is_not_silently_buffered() {
    let backend = new_backend().await;
    let mut req = argv(&["echo", "hi"]);
    req.stream = true;
    let err = backend.exec(req).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
}

#[tokio::test]
async fn a_timed_out_streamed_exec_releases_its_observer() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    // The stream ends when the observer's sender is dropped. A command cut short
    // by its timeout must not keep the observer alive inside the shell.
    let backend = new_backend().await;
    let mut req = argv(&["node", "-e", "let i=0; while(i<1000000000){i++}"]);
    req.timeout_ms = Some(50);
    let observer: OutputObserver = Arc::new(|_| {});
    let held = observer.clone();
    let err = backend.exec_streamed(req, observer).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Timeout);
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "the shell still holds the observer"
    );
}

#[tokio::test]
async fn exec_within_its_timeout_returns_the_output() {
    let backend = new_backend().await;
    let mut req = argv(&["echo", "quick"]);
    req.timeout_ms = Some(5_000);
    let out = backend.exec(req).await.unwrap();
    assert_eq!(out.stdout, b"quick\n".to_vec());
}

#[tokio::test]
async fn cpu_ms_counts_the_cpu_a_busy_command_uses() {
    let backend = new_backend().await;
    let busy = argv(&["sh", "-c", "i=0; while [ $i -lt 200 ]; do i=$((i+1)); done"]);
    let out = backend.exec(busy).await.unwrap();
    assert!(out.cpu_ms > 0, "a busy loop must report CPU time: {out:?}");
    // One command runs on one thread at a time, so its CPU cannot exceed its wall time.
    assert!(out.cpu_ms <= out.wall_ms + 1, "{out:?}");
}

#[tokio::test]
async fn exec_runs_a_command_and_returns_its_exit_code() {
    let backend = new_backend().await;
    let out = backend.exec(argv(&["echo", "hello"])).await.unwrap();
    assert_eq!(
        (out.exit_code, out.stdout.as_slice()),
        (0, b"hello\n".as_slice())
    );

    // A command the isolate tier cannot run is refused, not run as a failure.
    let missing = backend.exec(argv(&["nonexistent-tool"])).await.unwrap_err();
    assert_eq!(
        missing.code,
        sandbox_types::ErrorCode::NeedsVm,
        "{}",
        missing.message
    );
}

#[tokio::test]
async fn run_script_keeps_state_between_calls() {
    let backend = new_backend().await;
    backend
        .run_script("mkdir -p /work && cd /work")
        .await
        .unwrap();
    let out = backend.run_script("pwd && echo x > f").await.unwrap();
    assert_eq!(out.stdout, b"/work\n");
    // The file written by the first script is visible through the fs API.
    let h = backend
        .fs()
        .open(Path::new("/work/f"), OpenFlags::default())
        .await
        .unwrap();
    assert_eq!(backend.fs().read(&h, 0, 16).await.unwrap().as_ref(), b"x\n");
}

#[tokio::test]
async fn exec_applies_env_and_cwd_overrides() {
    let backend = new_backend().await;
    backend.run_script("mkdir /job").await.unwrap();
    let req = ExecRequest {
        argv: vec!["pwd".to_string()],
        env: HashMap::from([("TASK".to_string(), "1".to_string())]),
        cwd: Some("/job".to_string()),
        ..ExecRequest::default()
    };
    assert_eq!(backend.exec(req).await.unwrap().stdout, b"/job\n");
}

#[tokio::test]
async fn streaming_exec_is_refused_not_ignored() {
    let backend = new_backend().await;
    let streamed = ExecRequest {
        stream: true,
        ..argv(&["true"])
    };
    assert_eq!(
        backend.exec(streamed).await.err().unwrap().code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        backend
            .exec(ExecRequest::default())
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidArgument
    );
}

async fn write_text(fs: &dyn SessionFs, path: &str, text: &str) {
    let handle = fs
        .open(
            Path::new(path),
            OpenFlags {
                read: false,
                write: true,
                append: false,
                create: true,
                truncate: true,
                exclusive: false,
            },
        )
        .await
        .unwrap();
    fs.write(&handle, 0, text.as_bytes()).await.unwrap();
    fs.close(&handle);
}

async fn read_text(fs: &dyn SessionFs, path: &str) -> String {
    let handle = fs
        .open(Path::new(path), OpenFlags::default())
        .await
        .unwrap();
    let bytes = fs.read(&handle, 0, 1024).await.unwrap();
    fs.close(&handle);
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn checkpoint_restores_files_and_shell_state() {
    let backend = new_backend().await;
    write_text(backend.fs(), "/keep.txt", "kept").await;
    backend
        .fs()
        .mkdir(std::path::Path::new("/work"), 0o755)
        .await
        .unwrap();
    backend
        .run_script("export GREETING=hello; cd /work")
        .await
        .unwrap();
    let checkpoint = backend.checkpoint().await.unwrap();
    // Writes after the checkpoint belong to the session that is gone.
    write_text(backend.fs(), "/keep.txt", "later").await;

    let woken = restore_checkpoint(checkpoint).await.unwrap();
    assert_eq!(read_text(woken.fs(), "/keep.txt").await, "kept");
    let out = woken.run_script("echo $GREETING; pwd").await.unwrap();
    assert_eq!(out.stdout, b"hello\n/work\n");
}

#[tokio::test]
async fn restored_session_sees_the_snapshot_not_later_writes() {
    let backend = new_backend().await;
    write_text(backend.fs(), "/a.txt", "one").await;
    let root = backend.snapshot().await.unwrap();

    // Changes after the snapshot must not reach the restored session.
    write_text(backend.fs(), "/a.txt", "two").await;
    write_text(backend.fs(), "/later.txt", "new").await;

    let restored = restore(root).await.unwrap();
    assert_eq!(read_text(restored.fs(), "/a.txt").await, "one");
    assert!(restored.fs().stat(Path::new("/later.txt")).await.is_err());

    // The original keeps its own later state.
    assert_eq!(read_text(backend.fs(), "/a.txt").await, "two");
}
