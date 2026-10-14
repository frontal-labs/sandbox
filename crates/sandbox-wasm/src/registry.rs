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
//! ```
//!
//! The registry is data, read from the VFS. It does not grant new host
//! access: each tool runs under the same WASI host as any other module.

use crate::wasi::WasiLimits;
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
}

impl ToolSpec {
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

    /// Checks `module` against the pinned hash, if any.
    pub fn verify(&self, module: &[u8]) -> Result<(), SandboxError> {
        match self.expected_hash()? {
            Some(expected) if expected != ContentHash::new(module) => Err(SandboxError::new(
                ErrorCode::PermissionDenied,
                format!("tool {}: module does not match its pinned hash", self.name),
            )),
            _ => Ok(()),
        }
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
        };
        assert!(spec.verify(module).is_ok());
        let err = spec.verify(b"tampered").unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionDenied);
    }
}
