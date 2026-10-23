//! Runs the client against the real router on a local port, so each call goes
//! over HTTP to the same code the server runs.

use frontal_sandbox_sdk::Client;
use futures::StreamExt;
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::create_router;
use sandbox_types::{
    ErrorCode, ExecEvent, ExecRequest, OutputStream, SessionConfig, SessionId, SnapshotId,
    TenantId, Tier, TierPreference,
};
use std::sync::Arc;

/// Starts the server on a free port. Returns its base URL and a fresh key.
async fn start() -> (String, String) {
    // This server runs in the test process, which has no contained script runner. Scripts
    // run in-process here, as in the sandbox-server tests.
    let _ = sandbox_runtime::install_in_process_script_runner();
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), key)
}

async fn client() -> Client {
    let (base, key) = start().await;
    Client::new(base, Some(key))
}

fn config() -> SessionConfig {
    SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    }
}

#[tokio::test]
async fn session_exec_and_files_round_trip_over_http() {
    let client = client().await;
    let session = client.create_session(config()).await.unwrap();

    let out = client.exec(session, "echo hello").await.unwrap();
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout.trim(), "hello");

    client
        .write_file(session, "/note.txt", b"from the sdk".to_vec())
        .await
        .unwrap();
    assert_eq!(
        client.read_file(session, "/note.txt").await.unwrap(),
        b"from the sdk"
    );

    client.delete_file(session, "/note.txt").await.unwrap();
    let err = client.read_file(session, "/note.txt").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    client.destroy_session(session).await.unwrap();
    let err = client.exec(session, "echo gone").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn exec_stream_yields_output_then_exit_over_http() {
    let client = client().await;
    let session = client.create_session(config()).await.unwrap();
    let request = ExecRequest {
        argv: vec!["echo".to_string(), "streamed".to_string()],
        ..ExecRequest::default()
    };
    let events: Vec<ExecEvent> = client
        .exec_stream(session, &request)
        .await
        .unwrap()
        .map(|event| event.unwrap())
        .collect()
        .await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        events[0],
        ExecEvent::Output {
            stream: OutputStream::Stdout,
            data: b"streamed\n".to_vec(),
        }
    );
    assert!(matches!(events[1], ExecEvent::Exit { exit_code: 0, .. }));
}

#[tokio::test]
async fn exec_stream_ends_with_the_error_when_the_command_times_out() {
    let client = client().await;
    let session = client.create_session(config()).await.unwrap();
    let request = ExecRequest {
        argv: vec![
            "node".to_string(),
            "-e".to_string(),
            "let i=0; while(i<1000000000){i++}".to_string(),
        ],
        timeout_ms: Some(50),
        ..ExecRequest::default()
    };
    let events: Vec<ExecEvent> = client
        .exec_stream(session, &request)
        .await
        .unwrap()
        .map(|event| event.unwrap())
        .collect()
        .await;
    match events.last() {
        Some(ExecEvent::Error { code, .. }) => assert_eq!(*code, ErrorCode::Timeout),
        other => panic!("expected a timeout error line, got {other:?}"),
    }
}

#[tokio::test]
async fn snapshots_list_get_and_fork_over_http() {
    let client = client().await;
    let session = client.create_session(config()).await.unwrap();
    client
        .write_file(session, "/seed.txt", b"seed".to_vec())
        .await
        .unwrap();

    let snapshot: SnapshotId = client.snapshot(session).await.unwrap();

    let listed = client.list_snapshots(session).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, snapshot);

    let one = client.get_snapshot(session, snapshot).await.unwrap();
    assert_eq!(one.id, snapshot);
    assert_eq!(one.tier, Tier::Isolate);

    let forks: Vec<SessionId> = client.fork(snapshot, 2).await.unwrap();
    assert_eq!(forks.len(), 2);
    for fork in forks {
        assert_eq!(client.read_file(fork, "/seed.txt").await.unwrap(), b"seed");
    }
}

#[tokio::test]
async fn a_bad_key_is_refused_with_permission_denied() {
    let (base, _key) = start().await;
    let stranger = Client::new(base, Some("not-a-real-key".to_string()));
    let err = stranger.create_session(config()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn one_key_cannot_reach_another_keys_session() {
    let auth = Arc::new(AuthManager::new());
    let owner_key = auth.create_key();
    let other_key = auth.create_key();
    let app = create_router(Arc::new(Scheduler::new()), auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let owner = Client::new(base.clone(), Some(owner_key));
    let session = owner.create_session(config()).await.unwrap();

    let other = Client::new(base, Some(other_key));
    let err = other.exec(session, "echo nope").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn sleep_and_wake_restore_files_over_http() {
    let client = client().await;
    let session = client.create_session(config()).await.unwrap();
    client
        .write_file(session, "/kept.txt", b"before sleep".to_vec())
        .await
        .unwrap();

    let snapshot = client.sleep(session).await.unwrap();
    let state = client.get_session(session).await.unwrap();
    assert_eq!(state.status, sandbox_types::SessionStatus::Sleeping);

    // A command on a sleeping session wakes it first (ADR 0015).
    let out = client.exec(session, "echo hi").await.unwrap();
    assert_eq!(out.exit_code, 0);
    let state = client.get_session(session).await.unwrap();
    assert_eq!(state.status, sandbox_types::SessionStatus::Running);
    assert_eq!(
        client.read_file(session, "/kept.txt").await.unwrap(),
        b"before sleep"
    );

    // The sleep snapshot is an ordinary snapshot, so it can be forked.
    let forks = client.fork(snapshot, 1).await.unwrap();
    assert_eq!(
        client.read_file(forks[0], "/kept.txt").await.unwrap(),
        b"before sleep"
    );
}
