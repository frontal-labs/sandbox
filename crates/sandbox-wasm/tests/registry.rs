//! Integration tests for the tool registry manifest.

use sandbox_types::hash::ContentHash;
use sandbox_types::ErrorCode;
use sandbox_wasm::registry::{Registry, ToolSpec};
use sandbox_wasm::WasiLimits;

#[test]
fn parses_tools_and_applies_limit_overrides() {
    let registry = Registry::parse(
        r#"
[[tool]]
name = "wc2"
path = "/bin/wc2.wasm"
version = "1.0.0"
fuel = 5000
max_output_bytes = 64

[[tool]]
name = "plain"
path = "/bin/plain.wasm"
"#,
    )
    .unwrap();
    let wc2 = registry.find("wc2").unwrap();
    assert_eq!(wc2.limits().fuel, 5000);
    assert_eq!(wc2.limits().max_output_bytes, 64);
    assert_eq!(
        wc2.limits().max_memory_bytes,
        WasiLimits::default().max_memory_bytes
    );
    assert!(registry.find("missing").is_none());
}

#[test]
fn argv0_and_preopens_come_from_the_spec() {
    let registry = Registry::parse(
        "[[tool]]\nname = \"python\"\npath = \"/opt/python.wasm\"\nargv0 = \"/usr/bin/python\"\npreopen = [\"/usr\"]\n",
    )
    .unwrap();
    let python = registry.find("python").unwrap();
    let argv = vec!["python".to_string(), "-c".to_string(), "1".to_string()];
    assert_eq!(python.argv_for(&argv)[0], "/usr/bin/python");
    assert_eq!(python.argv_for(&argv)[1..], argv[1..]);
    let preopens = python.preopens();
    assert_eq!(preopens.len(), 1);
    assert!(preopens[0].readonly && preopens[0].guest_path == "/usr");
}

#[test]
fn refuses_a_relative_or_escaping_preopen() {
    for manifest in [
        "[[tool]]\nname = \"a\"\npath = \"/x\"\npreopen = [\"usr\"]",
        "[[tool]]\nname = \"a\"\npath = \"/x\"\npreopen = [\"/usr/../etc\"]",
        "[[tool]]\nname = \"a\"\npath = \"/x\"\nargv0 = \"python\"",
    ] {
        assert!(Registry::parse(manifest).is_err(), "{manifest}");
    }
}

#[test]
fn rejects_bad_manifests() {
    let cases = [
        "[[tool]]\nname = \"a/b\"\npath = \"/x\"",
        "[[tool]]\nname = \"a\"\npath = \"relative\"",
        "[[tool]]\nname = \"a\"\npath = \"/x\"\n[[tool]]\nname = \"a\"\npath = \"/y\"",
        "[[tool]]\nname = \"a\"\npath = \"/x\"\nhash = \"zz\"",
        "not = [valid",
    ];
    for text in cases {
        assert!(Registry::parse(text).is_err(), "should reject: {text}");
    }
}

#[test]
fn pinned_hash_must_match_the_module() {
    let module = b"module bytes";
    let good = ContentHash::new(module).to_hex();
    let spec = ToolSpec {
        name: "t".into(),
        path: "/bin/t.wasm".into(),
        version: None,
        hash: Some(good),
        fuel: None,
        max_memory_bytes: None,
        max_output_bytes: None,
        argv0: None,
        preopen: Vec::new(),
        reactor: None,
        reactor_hash: None,
    };
    assert!(spec.verify(module).is_ok());
    let err = spec.verify(b"tampered").unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

#[test]
fn a_reactor_is_checked_against_its_own_pin() {
    let reactor = b"reactor bytes";
    let spec = ToolSpec {
        name: "py".into(),
        path: "/opt/py.wasm".into(),
        version: None,
        hash: None,
        fuel: None,
        max_memory_bytes: None,
        max_output_bytes: None,
        argv0: None,
        preopen: Vec::new(),
        reactor: Some("/opt/py-init.wasm".into()),
        reactor_hash: Some(ContentHash::new(reactor).to_hex()),
    };
    assert!(spec.verify_reactor(reactor).is_ok());
    let err = spec.verify_reactor(b"tampered").unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    // The command module's pin does not cover the reactor, so it is unpinned here.
    assert!(spec.verify(b"anything").is_ok());
}
