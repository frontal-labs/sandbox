//! Replays the coreutils golden corpus (PLAN.md §4.4, §9 M2). Each case's expected
//! stdout and exit status were recorded from GNU coreutils by
//! `conformance/record_coreutils.py`. Every case runs on a fresh in-memory filesystem,
//! and every mismatch is reported, not only the first.

use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
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

/// Runs one script in a fresh sandbox and returns what it printed and its status.
async fn run(script: &str) -> Result<(String, i32), String> {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut shell = Shell::new(fs, ShellConfig::default());
    let out = shell.run(script).await.map_err(|e| e.message)?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.exit_code,
    ))
}

#[tokio::test]
async fn every_case_matches_gnu_coreutils() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/conformance/coreutils.json");
    let corpus: Corpus = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let total = corpus.cases.len();

    let mut failures = Vec::new();
    for case in &corpus.cases {
        match run(&case.script).await {
            Ok((stdout, status)) if stdout == case.stdout && status == case.status => {}
            Ok((stdout, status)) => failures.push(format!(
                "{}: script {:?}\n    gnu:     status {} stdout {:?}\n    sandbox: status {} stdout {:?}",
                case.name, case.script, case.status, case.stdout, status, stdout
            )),
            Err(message) => failures.push(format!(
                "{}: script {:?}\n    gnu:     status {} stdout {:?}\n    sandbox: error {message}",
                case.name, case.script, case.status, case.stdout
            )),
        }
    }
    let passed = total - failures.len();
    eprintln!(
        "coreutils: {passed}/{total} cases match {}",
        corpus.recorded_with
    );
    assert!(
        failures.is_empty(),
        "{} of {total} cases differ from GNU:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
