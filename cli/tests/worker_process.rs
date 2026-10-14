//! Runs the real `sandbox` binary as a worker process.

use sandbox_worker::Worker;

#[test]
fn worker_process_runs_scripts_and_survives_a_bad_request() {
    let binary = env!("CARGO_BIN_EXE_sandbox");
    let mut worker = Worker::spawn(binary, &["worker"]).unwrap();

    let ok = worker.run_script("echo hi | cat > /a && cat /a").unwrap();
    assert_eq!(ok.exit_code, 0);
    assert_eq!(ok.stdout, b"hi\n");

    // A script error is a normal failure response; the worker keeps running.
    let parse_error = worker.run_script("echo 'unterminated").unwrap_err();
    assert_eq!(parse_error.code, sandbox_types::ErrorCode::ShellError);

    let next = worker.run_script("node -e 'console.log(6 * 7)'").unwrap();
    assert_eq!(next.stdout, b"42\n");
    assert!(worker.is_alive());
    worker.shutdown();
}
