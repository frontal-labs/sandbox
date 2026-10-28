use super::invalid;
use super::time::{from_timestamp, to_timestamp};
use crate::v1::types as pb;
use sandbox_types::hash::ContentHash;
use sandbox_types::policy::{NetworkPolicy, ResourceLimits, SecretsConfig};
use sandbox_types::session::{
    CheckpointPolicy, SessionConfig, SessionState, SessionStatus, Snapshot, Tier, TierPreference,
};
use sandbox_types::{SandboxError, SnapshotId, TenantId};
use std::collections::HashMap;

impl From<CheckpointPolicy> for pb::CheckpointPolicy {
    fn from(policy: CheckpointPolicy) -> Self {
        match policy {
            CheckpointPolicy::PerExec => Self::PerExec,
            CheckpointPolicy::Manual => Self::Manual,
        }
    }
}

impl TryFrom<pb::CheckpointPolicy> for CheckpointPolicy {
    type Error = SandboxError;
    fn try_from(policy: pb::CheckpointPolicy) -> Result<Self, Self::Error> {
        match policy {
            pb::CheckpointPolicy::PerExec => Ok(Self::PerExec),
            pb::CheckpointPolicy::Manual => Ok(Self::Manual),
            pb::CheckpointPolicy::Unspecified => Ok(Self::default()),
        }
    }
}

impl From<Tier> for pb::Tier {
    fn from(tier: Tier) -> Self {
        match tier {
            Tier::Isolate => Self::Isolate,
            Tier::Vm => Self::Vm,
        }
    }
}

impl TryFrom<pb::Tier> for Tier {
    type Error = SandboxError;
    fn try_from(tier: pb::Tier) -> Result<Self, Self::Error> {
        match tier {
            pb::Tier::Unspecified => Err(invalid("tier is unspecified")),
            pb::Tier::Isolate => Ok(Self::Isolate),
            pb::Tier::Vm => Ok(Self::Vm),
        }
    }
}

impl From<TierPreference> for pb::TierPreference {
    fn from(pref: TierPreference) -> Self {
        match pref {
            TierPreference::Auto => Self::Auto,
            TierPreference::Isolate => Self::Isolate,
            TierPreference::Vm => Self::Vm,
        }
    }
}

impl TryFrom<pb::TierPreference> for TierPreference {
    type Error = SandboxError;
    fn try_from(pref: pb::TierPreference) -> Result<Self, Self::Error> {
        match pref {
            pb::TierPreference::Unspecified => Err(invalid("tier preference is unspecified")),
            pb::TierPreference::Auto => Ok(Self::Auto),
            pb::TierPreference::Isolate => Ok(Self::Isolate),
            pb::TierPreference::Vm => Ok(Self::Vm),
        }
    }
}

impl From<SessionStatus> for pb::SessionStatus {
    fn from(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Creating => Self::Creating,
            SessionStatus::Running => Self::Running,
            SessionStatus::Sleeping => Self::Sleeping,
            SessionStatus::Promoting => Self::Promoting,
            SessionStatus::Destroyed => Self::Destroyed,
            SessionStatus::Error => Self::Error,
        }
    }
}

impl TryFrom<pb::SessionStatus> for SessionStatus {
    type Error = SandboxError;
    fn try_from(status: pb::SessionStatus) -> Result<Self, SandboxError> {
        match status {
            pb::SessionStatus::Unspecified => Err(invalid("session status is unspecified")),
            pb::SessionStatus::Creating => Ok(Self::Creating),
            pb::SessionStatus::Running => Ok(Self::Running),
            pb::SessionStatus::Sleeping => Ok(Self::Sleeping),
            pb::SessionStatus::Promoting => Ok(Self::Promoting),
            pb::SessionStatus::Destroyed => Ok(Self::Destroyed),
            pb::SessionStatus::Error => Ok(Self::Error),
        }
    }
}

fn enum_i32<E: TryFrom<i32>>(value: i32, what: &str) -> Result<E, SandboxError> {
    E::try_from(value).map_err(|_| invalid(format!("unknown {what} value {value}")))
}

impl From<SessionConfig> for pb::SessionConfig {
    fn from(config: SessionConfig) -> Self {
        Self {
            tenant_id: Some(config.tenant_id.into()),
            tier_preference: pb::TierPreference::from(config.tier_preference) as i32,
            limits: Some(config.limits.into()),
            network: Some(config.network.into()),
            env: config.env.into_iter().collect(),
            cwd: config.cwd,
            initial_snapshot: config.initial_snapshot.map(Into::into),
            secrets: config.secrets.map(Into::into),
            labels: config.labels.into_iter().collect(),
            checkpoint: pb::CheckpointPolicy::from(config.checkpoint) as i32,
            checkpoint_retain: Some(config.checkpoint_retain),
        }
    }
}

impl TryFrom<pb::SessionConfig> for SessionConfig {
    type Error = SandboxError;
    fn try_from(config: pb::SessionConfig) -> Result<Self, Self::Error> {
        let tenant_id: TenantId = config
            .tenant_id
            .ok_or_else(|| invalid("tenant_id is required"))?
            .try_into()?;
        let network = match config.network {
            Some(network) => NetworkPolicy::try_from(network)?,
            None => NetworkPolicy::default(),
        };
        let initial_snapshot = config
            .initial_snapshot
            .map(SnapshotId::try_from)
            .transpose()?;
        Ok(Self {
            tenant_id,
            tier_preference: enum_i32::<pb::TierPreference>(
                config.tier_preference,
                "tier preference",
            )?
            .try_into()?,
            limits: config.limits.map(ResourceLimits::from).unwrap_or_default(),
            network,
            env: config.env.into_iter().collect::<HashMap<_, _>>(),
            cwd: config.cwd,
            initial_snapshot,
            secrets: config.secrets.map(SecretsConfig::from),
            labels: config.labels.into_iter().collect(),
            checkpoint: enum_i32::<pb::CheckpointPolicy>(config.checkpoint, "checkpoint policy")?
                .try_into()?,
            checkpoint_retain: config
                .checkpoint_retain
                .unwrap_or(sandbox_types::session::DEFAULT_CHECKPOINT_RETAIN),
        })
    }
}

impl From<SessionState> for pb::SessionState {
    fn from(state: SessionState) -> Self {
        Self {
            id: Some(state.id.into()),
            tenant_id: Some(state.tenant_id.into()),
            tier: pb::Tier::from(state.tier) as i32,
            status: pb::SessionStatus::from(state.status) as i32,
            config_hash: Some(state.config_hash.into()),
            vfs_root: state.vfs_root.map(Into::into),
            created_at: Some(to_timestamp(state.created_at)),
            updated_at: Some(to_timestamp(state.updated_at)),
            labels: state.labels.into_iter().collect(),
        }
    }
}

impl TryFrom<pb::SessionState> for SessionState {
    type Error = SandboxError;
    fn try_from(state: pb::SessionState) -> Result<Self, Self::Error> {
        Ok(Self {
            id: state
                .id
                .ok_or_else(|| invalid("id is required"))?
                .try_into()?,
            tenant_id: state
                .tenant_id
                .ok_or_else(|| invalid("tenant_id is required"))?
                .try_into()?,
            tier: enum_i32::<pb::Tier>(state.tier, "tier")?.try_into()?,
            status: enum_i32::<pb::SessionStatus>(state.status, "session status")?.try_into()?,
            config_hash: state
                .config_hash
                .ok_or_else(|| invalid("config_hash is required"))?
                .try_into()?,
            vfs_root: state.vfs_root.map(ContentHash::try_from).transpose()?,
            created_at: from_timestamp(
                &state
                    .created_at
                    .ok_or_else(|| invalid("created_at is required"))?,
            )?,
            updated_at: from_timestamp(
                &state
                    .updated_at
                    .ok_or_else(|| invalid("updated_at is required"))?,
            )?,
            labels: state.labels.into_iter().collect::<HashMap<_, _>>(),
        })
    }
}

impl From<Snapshot> for pb::Snapshot {
    fn from(snapshot: Snapshot) -> Self {
        Self {
            id: Some(snapshot.id.into()),
            parent: snapshot.parent.map(Into::into),
            vfs_root: Some(snapshot.vfs_root.into()),
            env: snapshot.env.into_iter().collect(),
            cwd: snapshot.cwd,
            tier: pb::Tier::from(snapshot.tier) as i32,
            runtime_blob: snapshot.runtime_blob.map(Into::into),
            created_at: Some(to_timestamp(snapshot.created_at)),
            labels: snapshot.labels.into_iter().collect(),
        }
    }
}

impl TryFrom<pb::Snapshot> for Snapshot {
    type Error = SandboxError;
    fn try_from(snapshot: pb::Snapshot) -> Result<Self, Self::Error> {
        Ok(Self {
            id: snapshot
                .id
                .ok_or_else(|| invalid("id is required"))?
                .try_into()?,
            parent: snapshot.parent.map(SnapshotId::try_from).transpose()?,
            vfs_root: snapshot
                .vfs_root
                .ok_or_else(|| invalid("vfs_root is required"))?
                .try_into()?,
            env: snapshot.env.into_iter().collect::<HashMap<_, _>>(),
            cwd: snapshot.cwd,
            tier: enum_i32::<pb::Tier>(snapshot.tier, "tier")?.try_into()?,
            runtime_blob: snapshot
                .runtime_blob
                .map(ContentHash::try_from)
                .transpose()?,
            created_at: from_timestamp(
                &snapshot
                    .created_at
                    .ok_or_else(|| invalid("created_at is required"))?,
            )?,
            labels: snapshot.labels.into_iter().collect::<HashMap<_, _>>(),
        })
    }
}
