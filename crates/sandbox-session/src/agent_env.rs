//! `/etc/agent/env.json` for Tier 1 sessions (IMAGE §4.5).
//!
//! The backend writes the file when it starts, so every session has one from
//! its creation, its restore, and its fork. Writing the same contents again
//! gives the same filesystem root, so a restore does not change the session.

use crate::filesystem::{vfs_error, write_path};
use crate::session::SessionFs;
use sandbox_types::agent_env::{
    AgentEnv, GuiInfo, ImageInfo, LimitInfo, NetworkInfo, AGENT_DIR, AGENT_ENV_PATH,
    AGENT_ENV_SCHEMA, AGENT_USER,
};
use sandbox_types::policy::PolicyAction;
use sandbox_types::{NetworkPolicy, SandboxError, SessionId, Tier};
use sandbox_vfs::VfsError;
use std::path::Path;

/// The agent commands a Tier 1 session has.
const COMMANDS: &[&str] = &["sandbox"];

/// Describes a Tier 1 session. `wall_timeout_s` is the command cap the tier
/// enforces. Memory and CPU are not enforced, so they are reported as `null`.
pub fn describe(id: SessionId, network: &NetworkPolicy, wall_timeout_s: u64) -> AgentEnv {
    AgentEnv {
        schema: AGENT_ENV_SCHEMA,
        session: id,
        tier: Tier::Isolate,
        user: AGENT_USER.to_string(),
        image: ImageInfo {
            base: None,
            layers: Vec::new(),
        },
        packages: Vec::new(),
        network: NetworkInfo {
            default: if network.default_deny {
                "deny"
            } else {
                "allow"
            }
            .to_string(),
            allow: network
                .rules
                .iter()
                .filter(|rule| rule.action == PolicyAction::Allow)
                .map(|rule| rule.host_pattern.clone())
                .collect(),
        },
        limits: LimitInfo {
            memory_mib: None,
            cpu_ms_budget: None,
            wall_timeout_s: Some(wall_timeout_s),
        },
        gui: GuiInfo {
            available: false,
            display: None,
            width: None,
            height: None,
        },
        commands: COMMANDS.iter().map(|c| (*c).to_string()).collect(),
    }
}

/// Writes `env` to `/etc/agent/env.json`, creating `/etc` and `/etc/agent` first
/// when they are missing.
pub async fn write(fs: &dyn SessionFs, env: &AgentEnv) -> Result<(), SandboxError> {
    for dir in ["/etc", AGENT_DIR] {
        match fs.mkdir(Path::new(dir), 0o755).await {
            Ok(()) | Err(VfsError::AlreadyExists(_)) => {}
            Err(error) => return Err(vfs_error(error)),
        }
    }
    let mut data = serde_json::to_vec_pretty(env)
        .map_err(|e| SandboxError::internal(format!("cannot encode {AGENT_ENV_PATH}: {e}")))?;
    data.push(b'\n');
    write_path(fs, AGENT_ENV_PATH, &data).await
}
