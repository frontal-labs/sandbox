//! Runs `sandbox bench` as a user would and checks its JSON output.

use std::process::Command;

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

#[test]
fn one_named_case_prints_one_json_line_with_ordered_percentiles() {
    let out = sandbox()
        .args(["bench", "--name", "exec_echo"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");

    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["name"], "exec_echo");
    assert_eq!(value["iterations"], 200);
    let p50 = value["p50_ns"].as_u64().unwrap();
    let p99 = value["p99_ns"].as_u64().unwrap();
    let max = value["max_ns"].as_u64().unwrap();
    let min = value["min_ns"].as_u64().unwrap();
    assert!(min <= p50 && p50 <= p99 && p99 <= max, "{stdout}");
    assert!(p50 > 0);
}

#[test]
fn an_unknown_case_fails_and_names_the_known_cases() {
    let out = sandbox()
        .args(["bench", "--name", "nope"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown benchmark nope"), "{stderr}");
    assert!(stderr.contains("snapshot_restore"), "{stderr}");
}
