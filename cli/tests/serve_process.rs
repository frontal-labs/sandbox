//! Starts `sandbox serve` with a data directory and checks the first-run key
//! rule and the state file, as a user sees them.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Kills the server when dropped, so a failing assertion never leaves it running.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// Starts the server and reads its startup lines up to `last`, the line that
/// closes what the test checks. The server prints the state directory after
/// it starts listening, and a first run prints the key after that.
fn start(dir: &Path, last: &str) -> (Server, Vec<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_frontal-sandbox"))
        .args(["serve", "--addr", &free_addr(), "--data-dir"])
        .arg(dir)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let server = Server(child);
    let mut lines = Vec::new();
    for line in BufReader::new(stderr).lines() {
        let line = line.unwrap();
        let done = line.starts_with(last);
        lines.push(line);
        if done {
            break;
        }
    }
    (server, lines)
}

#[test]
fn the_first_run_prints_a_key_and_a_restart_does_not() {
    let dir: PathBuf = std::env::temp_dir().join(format!("sandbox-serve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (first, lines) = start(&dir, "API key");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("API key (shown once): sbx_")),
        "{lines:?}"
    );
    drop(first);
    assert!(
        dir.join("state.json").exists(),
        "the state file is written at start"
    );

    let (second, lines) = start(&dir, "state directory");
    assert!(
        !lines.iter().any(|l| l.starts_with("API key")),
        "a restart must not print a new key: {lines:?}"
    );
    drop(second);
    let _ = std::fs::remove_dir_all(&dir);
}
