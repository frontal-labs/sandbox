//! Tool registry: a TOML manifest that maps command names to WASI modules.
//!
//! Format:
//! ```toml
//! [[tool]]
//! name = "wc2"
//! path = "/bin/wc2.wasm"                 # absolute VFS path
//! version = "1.0.0"                      # optional, informational
//! hash = "<blake3 hex of the module>"    # optional; checked before every run
//! fuel = 10000000                        # optional per-tool limits
//! max_memory_bytes = 16777216
//! max_output_bytes = 65536
//! argv0 = "/usr/bin/python"              # optional; replaces the name the shell gives
//! preopen = ["/usr"]                     # optional; read-only trees, mounted at the same path
//! ```
//!
//! The registry is data, read from the VFS. It does not grant new host
//! access: each tool runs under the same WASI host as any other module.

use crate::wasi::{Preopen, WasiLimits};
use sandbox_types::hash::ContentHash;
use sandbox_types::{ErrorCode, SandboxError};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub fuel: Option<u64>,
    #[serde(default)]
    pub max_memory_bytes: Option<usize>,
    #[serde(default)]
    pub max_output_bytes: Option<usize>,
    /// Absolute path the tool's argv[0] is set to. A runtime such as CPython finds its
    /// home from it, because the guest has no environment to say where it is.
    #[serde(default)]
    pub argv0: Option<String>,
    /// Absolute VFS trees the tool may read, each mounted read-only at the same path.
    #[serde(default)]
    pub preopen: Vec<String>,
    /// A reactor module for the `-c <script>` invocation: the same tool, with its interpreter
    /// initialised before it was snapshotted (see `wasi::runs_in_reactor`). The shell runs it
    /// for that invocation, and `path` for every other. Absent, `path` runs everything.
    #[serde(default)]
    pub reactor: Option<String>,
    /// The pinned hash of `reactor`, if any, checked like `hash`.
    #[serde(default)]
    pub reactor_hash: Option<String>,
}

impl ToolSpec {
    /// The argv a run gets: the shell's, with argv[0] replaced when the spec sets one.
    pub fn argv_for(&self, argv: &[String]) -> Vec<String> {
        let mut out = argv.to_vec();
        if let (Some(argv0), Some(first)) = (&self.argv0, out.first_mut()) {
            *first = argv0.clone();
        }
        out
    }

    /// The read-only preopens this tool runs with.
    pub fn preopens(&self) -> Vec<Preopen> {
        self.preopen
            .iter()
            .map(|path| Preopen {
                guest_path: path.clone(),
                root: path.clone(),
                readonly: true,
            })
            .collect()
    }

    /// Limits for this tool: the defaults, with the spec's overrides applied.
    pub fn limits(&self) -> WasiLimits {
        let defaults = WasiLimits::default();
        WasiLimits {
            fuel: self.fuel.unwrap_or(defaults.fuel),
            max_memory_bytes: self.max_memory_bytes.unwrap_or(defaults.max_memory_bytes),
            max_output_bytes: self.max_output_bytes.unwrap_or(defaults.max_output_bytes),
        }
    }

    /// The pinned hash, if the spec has one.
    pub fn expected_hash(&self) -> Result<Option<ContentHash>, SandboxError> {
        match &self.hash {
            None => Ok(None),
            Some(hex) => ContentHash::from_hex(hex)
                .map(Some)
                .map_err(|_| invalid(format!("tool {}: hash is not valid hex", self.name))),
        }
    }

    /// The reactor module to run for `argv`, when the tool has one and the invocation is one it
    /// serves. Otherwise `path` runs.
    pub fn reactor_for(&self, argv: &[String]) -> Option<&str> {
        if crate::wasi::runs_in_reactor(argv) {
            self.reactor.as_deref()
        } else {
            None
        }
    }

    /// Checks `module` against the pinned hash, if any.
    pub fn verify(&self, module: &[u8]) -> Result<(), SandboxError> {
        check_pinned(&self.name, self.expected_hash()?, module)
    }

    /// Checks the reactor module against its pinned hash, if any.
    pub fn verify_reactor(&self, module: &[u8]) -> Result<(), SandboxError> {
        let expected = match &self.reactor_hash {
            None => None,
            Some(hex) => Some(ContentHash::from_hex(hex).map_err(|_| {
                invalid(format!("tool {}: reactor_hash is not valid hex", self.name))
            })?),
        };
        check_pinned(&self.name, expected, module)
    }
}

/// Fails unless `module` has the pinned hash, when there is one.
fn check_pinned(
    name: &str,
    expected: Option<ContentHash>,
    module: &[u8],
) -> Result<(), SandboxError> {
    match expected {
        Some(expected) if expected != ContentHash::new(module) => Err(SandboxError::new(
            ErrorCode::PermissionDenied,
            format!("tool {name}: module does not match its pinned hash"),
        )),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Registry {
    #[serde(default, rename = "tool")]
    pub tools: Vec<ToolSpec>,
}

impl Registry {
    /// Parses and validates a manifest. Names must be unique, must not contain
    /// `/`, and paths must be absolute.
    pub fn parse(text: &str) -> Result<Self, SandboxError> {
        let registry: Registry =
            toml::from_str(text).map_err(|e| invalid(format!("tools manifest: {e}")))?;
        let mut seen = std::collections::BTreeSet::new();
        for tool in &registry.tools {
            if tool.name.is_empty() || tool.name.contains('/') {
                return Err(invalid(format!(
                    "tool name {:?} is not a plain name",
                    tool.name
                )));
            }
            if !seen.insert(tool.name.as_str()) {
                return Err(invalid(format!("tool {} is listed twice", tool.name)));
            }
            if !tool.path.starts_with('/') {
                return Err(invalid(format!(
                    "tool {}: path must be absolute",
                    tool.name
                )));
            }
            if let Some(argv0) = &tool.argv0 {
                if !argv0.starts_with('/') {
                    return Err(invalid(format!(
                        "tool {}: argv0 must be absolute",
                        tool.name
                    )));
                }
            }
            for tree in &tool.preopen {
                if !tree.starts_with('/')
                    || tree.len() < 2
                    || tree.contains("/../")
                    || tree.ends_with("/..")
                {
                    return Err(invalid(format!(
                        "tool {}: preopen {tree:?} must be an absolute path",
                        tool.name
                    )));
                }
            }
            tool.expected_hash()?;
        }
        Ok(registry)
    }

    pub fn find(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

fn invalid(message: String) -> SandboxError {
    SandboxError::new(ErrorCode::InvalidArgument, message)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
