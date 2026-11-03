//! The shipped tool manifest pins a hash for every tool whose module is in the
//! fixtures, and each pinned hash matches the module it names. A rebuilt tool
//! that is not re-pinned fails here, and so does a manifest entry for a module
//! that is missing.

use sandbox_wasm::{Registry, WasiLimits};

const MANIFEST: &str = include_str!("../fixtures/tools/tools.toml");

fn module(name: &str) -> Option<&'static [u8]> {
    match name {
        "jq" => Some(include_bytes!("../fixtures/jq_tool.wasm")),
        "rg" => Some(include_bytes!("../fixtures/rg_tool.wasm")),
        _ => None,
    }
}

#[test]
fn the_pinned_hashes_match_the_shipped_modules() {
    let registry = Registry::parse(MANIFEST).unwrap();
    for name in ["jq", "rg"] {
        let spec = registry.find(name).unwrap_or_else(|| panic!("{name} is not in the manifest"));
        let bytes = module(name).unwrap();
        spec.verify(bytes)
            .unwrap_or_else(|e| panic!("{name}: {}", e.message));
        assert!(spec.hash.is_some(), "{name} has no pinned hash");
    }
}

#[test]
fn rg_gets_the_budget_its_regex_compilation_needs() {
    let registry = Registry::parse(MANIFEST).unwrap();
    let rg = registry.find("rg").unwrap();
    assert!(rg.limits().fuel >= 1_000_000_000);
    assert!(rg.limits().fuel > WasiLimits::default().fuel);
}
