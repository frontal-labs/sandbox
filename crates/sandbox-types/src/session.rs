use crate::hash::ContentHash;
use crate::ids::{SnapshotId, TenantId};
use crate::policy::{NetworkPolicy, ResourceLimits, SecretsConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TierPreference {
    #[default]
    Auto,
    Isolate,
    Vm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionConfig {
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
    pub tenant_id: TenantId,
    pub tier_preference: TierPreference,
    pub limits: ResourceLimits,
    pub network: NetworkPolicy,
    pub env: HashMap<String, String>,
    pub cwd: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionState {
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
    pub id: crate::ids::SessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
    pub tenant_id: TenantId,
    pub tier: Tier,
    pub status: SessionStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9a-f]{64}$"))]
    pub config_hash: ContentHash,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, pattern = "^[0-9a-f]{64}$"))]
    pub vfs_root: Option<ContentHash>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: SystemTime,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: SystemTime,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SessionStatus {
    Creating,
    Running,
    Sleeping,
    Promoting,
    Destroyed,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExecRequest {
    pub argv: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    /// Bytes as an integer array, 0-255 each (serde default).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<u32>>))]
    pub stdin: Option<Vec<u8>>,
    pub timeout_ms: Option<u64>,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExecResult {
    pub exit_code: i32,
    /// Bytes as an integer array, 0-255 each (serde default).
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<u32>))]
    pub stdout: Vec<u8>,
    /// Bytes as an integer array, 0-255 each (serde default).
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<u32>))]
    pub stderr: Vec<u8>,
    pub cpu_ms: u64,
    pub wall_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Snapshot {
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
    pub id: SnapshotId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"))]
    pub parent: Option<SnapshotId>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, pattern = "^[0-9a-f]{64}$"))]
    pub vfs_root: ContentHash,
    pub env: HashMap<String, String>,
    pub cwd: String,
    pub tier: Tier,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, pattern = "^[0-9a-f]{64}$"))]
    pub runtime_blob: Option<ContentHash>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: SystemTime,
    pub labels: HashMap<String, String>,
}
