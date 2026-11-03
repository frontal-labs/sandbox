//! The sandboxed `sqlite3` (`fixtures/sqlite3_tool.wasm`, built from `tools/wasm/sqlite`)
//! matches the system SQLite CLI on the cases in `tests/data/sqlite_cases.json`, byte for
//! byte. The file-database round trip checks that a database written in one run is read
//! back by the next, through the VFS.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{SessionFs, Vfs};
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

const SQLITE_TOOL: &[u8] = include_bytes!("../fixtures/sqlite3_tool.wasm");
const CASES: &str = include_str!("data/sqlite_cases.json");

#[derive(Deserialize)]
struct Case {
    flags: Vec<String>,
    sql: String,
    expected: String,
    status: i32,
}

fn limits() -> WasiLimits {
    WasiLimits {
        fuel: 1_000_000_000,
        ..WasiLimits::default()
    }
}

async fn run(vfs: &Arc<Vfs>, argv: Vec<String>, stdin: Vec<u8>) -> sandbox_wasm::WasiOutcome {
    run_module_with_mount(
        SQLITE_TOOL.to_vec(),
        argv,
        stdin,
        limits(),
        Some(Mount {
            vfs: vfs.clone(),
            root: "/work".to_string(),
        }),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn the_sandboxed_sqlite_matches_the_reference_cli() {
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    assert!(cases.len() >= 9, "the corpus shrank to {}", cases.len());
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    let mut failures = Vec::new();
    for case in &cases {
        let mut argv = vec!["sqlite3".to_string()];
        argv.extend(case.flags.iter().cloned());
        argv.push(":memory:".to_string());
        argv.push(case.sql.clone());
        let out = run(&vfs, argv, Vec::new()).await;
        let got = String::from_utf8_lossy(&out.stdout).into_owned();
        if got != case.expected || out.exit_code != case.status {
            failures.push(format!(
                "{:?} -> {:?} (exit {}), expected {:?} (exit {}); stderr {:?}",
                case.sql,
                got,
                out.exit_code,
                case.expected,
                case.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn a_file_database_persists_between_runs() {
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    // The guest sees its mount root as /, so the file is /work/roundtrip.sqlite on the host.
    let db = "/roundtrip.sqlite".to_string();

    let write = run(
        &vfs,
        vec![
            "sqlite3".into(),
            db.clone(),
            "create table k(v); insert into k values(42);".into(),
        ],
        Vec::new(),
    )
    .await;
    assert_eq!(
        write.exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );

    let read = run(
        &vfs,
        vec!["sqlite3".into(), db, "select v from k;".into()],
        Vec::new(),
    )
    .await;
    assert_eq!(
        read.exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&read.stdout), "42\n");
}

#[tokio::test]
async fn an_sql_error_is_reported_on_stderr_with_exit_status_one() {
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    let out = run(
        &vfs,
        vec![
            "sqlite3".into(),
            ":memory:".into(),
            "select * from nope;".into(),
        ],
        Vec::new(),
    )
    .await;
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("Error: no such table: nope"));
}
