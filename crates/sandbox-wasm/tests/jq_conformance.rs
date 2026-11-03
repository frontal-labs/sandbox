//! The sandboxed `jq` (`fixtures/jq_tool.wasm`, built from `tools/wasm/jq`) matches
//! reference `jq` 1.7.1 output on every case in `tests/data/jq_cases.json`. The
//! cases were recorded with the system `jq`, and each one is a filter the two
//! implementations agree on. Output must match byte for byte.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{SessionFs, Vfs};
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits};
use serde::Deserialize;
use std::sync::Arc;

const JQ_TOOL: &[u8] = include_bytes!("../fixtures/jq_tool.wasm");
const CASES: &str = include_str!("data/jq_cases.json");

#[derive(Deserialize)]
struct Case {
    filter: String,
    input: String,
    flags: Vec<String>,
    expected: String,
    status: i32,
}

#[tokio::test]
async fn the_sandboxed_jq_matches_reference_jq() {
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    assert!(cases.len() >= 40, "the corpus shrank to {}", cases.len());
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    vfs.mkdir(std::path::Path::new("/work"), 0o755)
        .await
        .unwrap();

    let mut failures = Vec::new();
    for case in &cases {
        let mut argv = vec!["jq".to_string()];
        argv.extend(case.flags.iter().cloned());
        argv.push(case.filter.clone());
        let out = run_module_with_mount(
            JQ_TOOL.to_vec(),
            argv,
            case.input.as_bytes().to_vec(),
            WasiLimits::default(),
            Some(Mount {
                vfs: vfs.clone(),
                root: "/work".to_string(),
            }),
        )
        .await
        .unwrap();
        let got = String::from_utf8_lossy(&out.stdout).into_owned();
        if got != case.expected || out.exit_code != case.status {
            failures.push(format!(
                "{:?} {:?} -> {:?} (exit {}), expected {:?} (exit {}); stderr {:?}",
                case.flags,
                case.filter,
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
