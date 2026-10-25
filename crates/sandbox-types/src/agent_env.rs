//! The machine-readable environment file, `/etc/agent/env.json` (IMAGE §4.5).
//!
//! Schema version 1. A field a tier does not have, or does not enforce, is
//! `null` rather than a guessed value. The file never holds environment values
//! or secrets, because those are kept out of the filesystem (PLAN §14).

use crate::ids::SessionId;
use crate::session::Tier;
use serde::{Deserialize, Serialize};

/// Where the environment file lives in the session's filesystem.
pub const AGENT_ENV_PATH: &str = "/etc/agent/env.json";

/// The directory that holds the environment file.
pub const AGENT_DIR: &str = "/etc/agent";

/// The user an agent's commands run as, in both tiers (IMAGE §4.5).
pub const AGENT_USER: &str = "agent";

/// The schema version this build writes.
pub const AGENT_ENV_SCHEMA: u32 = 1;

/// The contents of `/etc/agent/env.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEnv {
    pub schema: u32,
    /// The session's ID, as the REST API spells it.
    pub session: SessionId,
    pub tier: Tier,
    /// The user the session's commands run as. Tier 1 has no users, and it reports
    /// the name the VM tier uses, so an agent sees one contract in both tiers.
    pub user: String,
    pub image: ImageInfo,
    pub packages: Vec<PackageInfo>,
    pub network: NetworkInfo,
    pub limits: LimitInfo,
    pub gui: GuiInfo,
    /// The agent commands this session has, by name.
    pub commands: Vec<String>,
}

/// The image the session was built from. Tier 1 has no layers, so both fields
/// are empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub base: Option<String>,
    pub layers: Vec<String>,
}

/// A package the session's image provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
}

/// The session's network policy. The default is to deny everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInfo {
    /// `"deny"` or `"allow"`.
    pub default: String,
    /// The host patterns the policy allows.
    pub allow: Vec<String>,
}

/// The limits the session runs under. A limit the tier does not enforce is `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitInfo {
    pub memory_mib: Option<u64>,
    pub cpu_ms_budget: Option<u64>,
    pub wall_timeout_s: Option<u64>,
}

/// The session's desktop, if it has one. Tier 1 has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiInfo {
    pub available: bool,
    pub display: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentEnv {
        AgentEnv {
            schema: AGENT_ENV_SCHEMA,
            session: SessionId::from_ulid(
                ulid::Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            ),
            tier: Tier::Isolate,
            user: "agent".to_string(),
            image: ImageInfo {
                base: None,
                layers: Vec::new(),
            },
            packages: Vec::new(),
            network: NetworkInfo {
                default: "deny".to_string(),
                allow: Vec::new(),
            },
            limits: LimitInfo {
                memory_mib: None,
                cpu_ms_budget: None,
                wall_timeout_s: Some(300),
            },
            gui: GuiInfo {
                available: false,
                display: None,
                width: None,
                height: None,
            },
            commands: vec!["sandbox".to_string()],
        }
    }

    #[test]
    fn serializes_to_the_documented_keys() {
        let value = serde_json::to_value(sample()).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["session"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(value["tier"], "isolate");
        assert_eq!(value["user"], "agent");
        assert_eq!(value["image"]["base"], serde_json::Value::Null);
        assert_eq!(value["limits"]["memory_mib"], serde_json::Value::Null);
        assert_eq!(value["limits"]["wall_timeout_s"], 300);
        assert_eq!(value["gui"]["available"], false);
        assert_eq!(value["commands"][0], "sandbox");
    }

    #[test]
    fn round_trips_through_json() {
        let env = sample();
        let text = serde_json::to_string(&env).unwrap();
        let back: AgentEnv = serde_json::from_str(&text).unwrap();
        assert_eq!(back, env);
    }
}
