//! Replays the conformance corpus (PLAN.md §7, §9 M1). Each case's expected stdout
//! and exit status were recorded from GNU bash by `conformance/record.py`. Every
//! case runs here on a fresh in-memory filesystem, and every mismatch is reported,
//! not only the first.

use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::Vfs;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct Corpus {
    recorded_with: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    script: String,
    stdout: String,
    status: i32,
}

/// Runs one script in a fresh sandbox and returns what it printed and its status,
/// or the error that stopped it before it ran.
async fn run(script: &str) -> Result<(String, i32), SandboxError> {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut shell = Shell::new(fs, ShellConfig::default());
    let out = shell.run(script).await?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.exit_code,
    ))
}

#[tokio::test]
async fn every_case_matches_bash() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/conformance/cases.json");
    let corpus: Corpus = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let total = corpus.cases.len();
    assert!(
        total >= 200,
        "the corpus has {total} cases; PLAN §9 M1 needs 200"
    );

    let mut failures = Vec::new();
    for case in &corpus.cases {
        match run(&case.script).await {
            Ok((stdout, status)) if stdout == case.stdout && status == case.status => {}
            Ok((stdout, status)) => failures.push(format!(
                "{}: script {:?}\n    bash:    status {} stdout {:?}\n    sandbox: status {} stdout {:?}",
                case.name, case.script, case.status, case.stdout, status, stdout
            )),
            // A syntax error is an error from `run`, not an exit status. Bash reports
            // it with status 2 and no output, and the sandbox refuses the script
            // before it runs anything. That is the same outcome.
            Err(error)
                if error.code == ErrorCode::ShellError
                    && case.status == 2
                    && case.stdout.is_empty() => {}
            Err(error) => failures.push(format!(
                "{}: script {:?}\n    bash:    status {} stdout {:?}\n    sandbox: error {}",
                case.name, case.script, case.status, case.stdout, error.message
            )),
        }
    }
    let passed = total - failures.len();
    eprintln!(
        "conformance: {passed}/{total} cases match {}",
        corpus.recorded_with
    );
    assert!(
        failures.is_empty(),
        "{} of {total} cases differ from bash:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
