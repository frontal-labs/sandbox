use crate::hash::ContentHash;
use crate::ids::{SnapshotId, TenantId};
use crate::policy::{NetworkPolicy, ResourceLimits, SecretsConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Isolate,
    Vm,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Isolate => "isolate",
            Tier::Vm => "vm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TierPreference {
    #[default]
    Auto,
    Isolate,
    Vm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub tenant_id: TenantId,
    pub tier_preference: TierPreference,
    pub limits: ResourceLimits,
    pub network: NetworkPolicy,
    pub env: HashMap<String, String>,
    pub cwd: String,
    pub initial_snapshot: Option<SnapshotId>,
    pub secrets: Option<SecretsConfig>,
    pub labels: HashMap<String, String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            tenant_id: TenantId::nil(),
            tier_preference: TierPreference::default(),
            limits: ResourceLimits::default(),
            network: NetworkPolicy::default(),
            env: HashMap::new(),
            cwd: "/work".to_string(),
            initial_snapshot: None,
            secrets: None,
            labels: HashMap::new(),
        }
    }
}

impl SessionConfig {
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            ..Default::default()
        }
    }

    pub fn with_tier(mut self, tier: TierPreference) -> Self {
        self.tier_preference = tier;
        self
    }

    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_network(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn with_snapshot(mut self, snapshot: SnapshotId) -> Self {
        self.initial_snapshot = Some(snapshot);
        self
    }

    pub fn with_secrets(mut self, secrets: SecretsConfig) -> Self {
        self.secrets = Some(secrets);
        self
    }

    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub id: crate::ids::SessionId,
    pub tenant_id: TenantId,
    pub tier: Tier,
    pub status: SessionStatus,
    pub config_hash: ContentHash,
    pub vfs_root: Option<ContentHash>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Creating,
    Running,
    Sleeping,
    Promoting,
    Destroyed,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub argv: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub stdin: Option<Vec<u8>>,
    pub timeout_ms: Option<u64>,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub cpu_ms: u64,
    pub wall_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub parent: Option<SnapshotId>,
    pub vfs_root: ContentHash,
    pub env: HashMap<String, String>,
    pub cwd: String,
    pub tier: Tier,
    pub runtime_blob: Option<ContentHash>,
    pub created_at: SystemTime,
    pub labels: HashMap<String, String>,
}
