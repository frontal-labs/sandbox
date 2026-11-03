//! The sandboxed `rg` (`fixtures/rg_tool.wasm`, built from `tools/wasm/rg`) matches
//! ripgrep 15.2 on stdin searches in `tests/data/rg_cases.json`, byte for byte, with
//! the same exit status.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits};
use serde::Deserialize;
use std::sync::Arc;

const RG_TOOL: &[u8] = include_bytes!("../fixtures/rg_tool.wasm");
const CASES: &str = include_str!("data/rg_cases.json");

#[derive(Deserialize)]
struct Case {
    flags: Vec<String>,
    input: String,
    expected: String,
    status: i32,
}

#[tokio::test]
async fn the_sandboxed_rg_matches_ripgrep_on_stdin() {
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    assert!(cases.len() >= 15, "the corpus shrank to {}", cases.len());
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));

    let mut failures = Vec::new();
    for case in &cases {
        let mut argv = vec!["rg".to_string()];
        argv.extend(case.flags.iter().cloned());
        let out = run_module_with_mount(
            RG_TOOL.to_vec(),
            argv,
            case.input.as_bytes().to_vec(),
            // The budget tools.toml gives rg: compiling the Unicode word class is expensive.
            WasiLimits {
                fuel: 1_000_000_000,
                ..WasiLimits::default()
            },
            Some(Mount {
                vfs: vfs.clone(),
                root: "/".to_string(),
            }),
        )
        .await
        .unwrap();
        let got = String::from_utf8_lossy(&out.stdout).into_owned();
        if got != case.expected || out.exit_code != case.status {
            failures.push(format!(
                "{:?} -> {:?} (exit {}), expected {:?} (exit {}); stderr {:?}",
                case.flags,
                got,
                out.exit_code,
                case.expected,
                case.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
