//! Integration tests for filesystem replay, moved out of src/filesystem.rs.

use sandbox_session::{replay_run, IsolateBackend, Session};
use sandbox_trace::{Outcome, RunRecord};
use sandbox_types::SessionConfig;

#[tokio::test]
async fn a_replayed_run_draws_the_same_random_values() {
    let original = IsolateBackend::new(SessionConfig::default()).await.unwrap();
    let state = original.random_state().await;
    let out = original.run_script("echo $RANDOM $RANDOM").await.unwrap();
    let run = RunRecord {
        seq: 1,
        script: "echo $RANDOM $RANDOM".to_string(),
        fs: None,
        outcome: Outcome::Completed {
            exit_code: out.exit_code,
            stdout: out.stdout.clone(),
            stderr: out.stderr.clone(),
        },
        files: Vec::new(),
        random_state: Some(state),
    };

    // A fresh session starts from a different clock seed, so it would draw other
    // values without the recorded state.
    let fresh = IsolateBackend::new(SessionConfig::default()).await.unwrap();
    let (code, stdout, _) = replay_run(&fresh, &run).await.unwrap();
    assert_eq!(code, out.exit_code);
    assert_eq!(
        stdout, out.stdout,
        "the replay reproduces the recorded $RANDOM values"
    );
}
