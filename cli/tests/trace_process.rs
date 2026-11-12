//! Records and replays traces with the real `sandbox` binary, so the exit codes
//! and the files it writes are checked as a user sees them.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// Path to the sandbox binary. Prefers the canonical `sandbox` name, falling
/// back to the legacy `frontal-sandbox` name. Cargo sets both; Bazel sets only
/// the ones listed in `env` (cli/BUILD.bazel).
fn sandbox_path() -> String {
    std::env::var("CARGO_BIN_EXE_sandbox")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_frontal-sandbox"))
        .expect("Cargo/Bazel should provide a sandbox test binary path")
}

fn sandbox() -> Command {
    Command::new(sandbox_path())
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sandbox-trace-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_recorded_run_replays_with_no_divergence() {
    let dir = scratch("faithful");
    let trace = dir.join("run.jsonl");

    let run = sandbox()
        .args(["run", "--trace"])
        .arg(&trace)
        .arg("echo recorded")
        .output()
        .unwrap();
    assert!(run.status.success());
    assert_eq!(text(&run), "recorded\n");

    let replay = sandbox().arg("replay").arg(&trace).output().unwrap();
    assert_eq!(replay.status.code(), Some(0), "{}", text(&replay));
    assert!(text(&replay).contains("replayed 1 runs: 0 diverged"));
}

#[test]
fn a_changed_output_is_reported_and_exits_one() {
    let dir = scratch("tampered");
    let trace = dir.join("run.jsonl");
    assert!(sandbox()
        .args(["run", "--trace"])
        .arg(&trace)
        .arg("echo original")
        .status()
        .unwrap()
        .success());

    // Replace the recorded stdout ("original\n") with "changed\n".
    let recorded = std::fs::read_to_string(&trace).unwrap();
    let changed = recorded.replace("6f726967696e616c0a", "6368616e6765640a");
    assert_ne!(
        recorded, changed,
        "the recorded output should be the hex of original"
    );
    std::fs::write(&trace, changed).unwrap();

    let replay = sandbox().arg("replay").arg(&trace).output().unwrap();
    assert_eq!(replay.status.code(), Some(1));
    let report = text(&replay);
    assert!(report.contains("1 diverged"), "{report}");
    assert!(
        report.contains("expected: exit 0, stdout \"changed"),
        "{report}"
    );
    assert!(
        report.contains("actual:   exit 0, stdout \"original"),
        "{report}"
    );
}

#[test]
fn a_shell_session_replays_its_file_state_in_order() {
    let dir = scratch("shell");
    let trace = dir.join("shell.jsonl");
    let mut shell = sandbox()
        .args(["shell", "--trace"])
        .arg(&trace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    shell
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo one > /f\ncat /f\nexit\n")
        .unwrap();
    let out = shell.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(text(&out), "one\n");

    let replay = sandbox().arg("replay").arg(&trace).output().unwrap();
    assert_eq!(replay.status.code(), Some(0), "{}", text(&replay));
    assert!(text(&replay).contains("replayed 2 runs: 0 diverged"));
}

#[test]
fn a_file_that_is_not_a_trace_is_refused() {
    let dir = scratch("garbage");
    let file = dir.join("not-a-trace.jsonl");
    std::fs::write(&file, "hello\n").unwrap();
    let replay = sandbox().arg("replay").arg(&file).output().unwrap();
    assert_eq!(replay.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&replay.stderr).contains("not a sandbox-trace header"));
}

#[test]
fn control_characters_from_a_trace_are_escaped_in_the_report() {
    // A trace is a file that can come from anywhere. Its script carries an
    // escape sequence and its recorded output is wrong, so the report prints
    // the script. The raw ESC byte must not reach the terminal.
    let dir = scratch("escapes");
    let trace = dir.join("hostile.jsonl");
    std::fs::write(
        &trace,
        concat!(
            r#"{"format":"sandbox-trace","version":1}"#,
            "\n",
            r#"{"seq":1,"script":"echo \u001b]0;pwned\u0007","exit_code":0,"stdout":"6e6f","stderr":""}"#,
            "\n",
        ),
    )
    .unwrap();

    let replay = sandbox().arg("replay").arg(&trace).output().unwrap();
    assert_eq!(replay.status.code(), Some(1));
    let report = replay.stdout;
    assert!(!report.contains(&0x1b), "raw escape byte in the report");
    assert!(!report.contains(&0x07), "raw bell byte in the report");
    assert!(String::from_utf8_lossy(&report).contains(r"\u{1b}"));
}

#[test]
fn random_values_in_a_recorded_run_replay_identically() {
    // Each run starts from a fresh session, which seeds $RANDOM from the clock. The trace
    // records the generator state, so a replay draws the same values.
    let dir = scratch("random");
    let trace = dir.join("run.jsonl");

    let run = sandbox()
        .args(["run", "--trace"])
        .arg(&trace)
        .arg("echo $RANDOM $RANDOM $RANDOM")
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", text(&run));

    let replay = sandbox().arg("replay").arg(&trace).output().unwrap();
    assert_eq!(replay.status.code(), Some(0), "{}", text(&replay));
    assert!(
        text(&replay).contains("replayed 1 runs: 0 diverged"),
        "{}",
        text(&replay)
    );
}
