use super::invalid;
use crate::v1::types as pb;
use sandbox_types::policy::{
    Capability, NetworkPolicy, NetworkPolicyRule, PolicyAction, ResourceLimits, SecretEntry,
    SecretsConfig,
};
use sandbox_types::SandboxError;
use std::collections::HashMap;

impl From<PolicyAction> for pb::PolicyAction {
    fn from(action: PolicyAction) -> Self {
        match action {
            PolicyAction::Allow => Self::Allow,
            PolicyAction::Deny => Self::Deny,
        }
    }
}

impl TryFrom<pb::PolicyAction> for PolicyAction {
    type Error = SandboxError;
    fn try_from(action: pb::PolicyAction) -> Result<Self, Self::Error> {
        match action {
            pb::PolicyAction::Unspecified => Err(invalid("policy action is unspecified")),
            pb::PolicyAction::Allow => Ok(Self::Allow),
            pb::PolicyAction::Deny => Ok(Self::Deny),
        }
    }
}

impl From<ResourceLimits> for pb::ResourceLimits {
    fn from(l: ResourceLimits) -> Self {
        Self {
            memory_bytes: l.memory_bytes,
            cpu_ms_per_sec: l.cpu_ms_per_sec,
            wall_timeout_secs: l.wall_timeout_secs,
            max_file_size: l.max_file_size,
            max_total_size: l.max_total_size,
            max_inodes: l.max_inodes,
            max_path_depth: l.max_path_depth,
            max_processes: l.max_processes,
            max_open_files: l.max_open_files,
        }
    }
}

impl From<pb::ResourceLimits> for ResourceLimits {
    fn from(l: pb::ResourceLimits) -> Self {
        Self {
            memory_bytes: l.memory_bytes,
            cpu_ms_per_sec: l.cpu_ms_per_sec,
            wall_timeout_secs: l.wall_timeout_secs,
            max_file_size: l.max_file_size,
            max_total_size: l.max_total_size,
            max_inodes: l.max_inodes,
            max_path_depth: l.max_path_depth,
            max_processes: l.max_processes,
            max_open_files: l.max_open_files,
        }
    }
}

impl From<NetworkPolicyRule> for pb::NetworkPolicyRule {
    fn from(rule: NetworkPolicyRule) -> Self {
        Self {
            action: pb::PolicyAction::from(rule.action) as i32,
            host_pattern: rule.host_pattern,
            ports: rule.ports.map(|ports| pb::PortList {
                ports: ports.into_iter().map(u32::from).collect(),
            }),
        }
    }
}

impl TryFrom<pb::NetworkPolicyRule> for NetworkPolicyRule {
    type Error = SandboxError;
    fn try_from(rule: pb::NetworkPolicyRule) -> Result<Self, Self::Error> {
        let action = PolicyAction::try_from(
            pb::PolicyAction::try_from(rule.action)
                .map_err(|_| invalid("unknown policy action"))?,
        )?;
        let ports = match rule.ports {
            None => None,
            Some(list) => Some(
                list.ports
                    .into_iter()
                    .map(|port| {
                        u16::try_from(port)
                            .map_err(|_| invalid(format!("port {port} out of range")))
                    })
                    .collect::<Result<Vec<u16>, _>>()?,
            ),
        };
        Ok(Self {
            action,
            host_pattern: rule.host_pattern,
            ports,
        })
    }
}

impl From<NetworkPolicy> for pb::NetworkPolicy {
    fn from(policy: NetworkPolicy) -> Self {
        Self {
            default_deny: policy.default_deny,
            rules: policy
                .rules
                .into_iter()
                .map(pb::NetworkPolicyRule::from)
                .collect(),
        }
    }
}

impl TryFrom<pb::NetworkPolicy> for NetworkPolicy {
    type Error = SandboxError;
    fn try_from(policy: pb::NetworkPolicy) -> Result<Self, Self::Error> {
        Ok(Self {
            default_deny: policy.default_deny,
            rules: policy
                .rules
                .into_iter()
                .map(NetworkPolicyRule::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<SecretsConfig> for pb::SecretsConfig {
    fn from(config: SecretsConfig) -> Self {
        Self {
            secrets: config
                .secrets
                .into_iter()
                .map(|(name, entry)| {
                    (
                        name,
                        pb::SecretEntry {
                            value: entry.value,
                            hosts: entry.hosts,
                        },
                    )
                })
                .collect(),
        }
    }
}

impl From<pb::SecretsConfig> for SecretsConfig {
    fn from(config: pb::SecretsConfig) -> Self {
        Self {
            secrets: config
                .secrets
                .into_iter()
                .map(|(name, entry)| {
                    (
                        name,
                        SecretEntry {
                            value: entry.value,
                            hosts: entry.hosts,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
        }
    }
}

impl From<Capability> for pb::Capability {
    fn from(cap: Capability) -> Self {
        use pb::capability::Kind;
        let kind = match cap {
            Capability::FsRead(v) => Kind::FsRead(v),
            Capability::FsWrite(v) => Kind::FsWrite(v),
            Capability::NetHttp(v) => Kind::NetHttp(v),
            Capability::NetTcp(v) => Kind::NetTcp(v),
            Capability::Exec(v) => Kind::Exec(v),
            Capability::Env(v) => Kind::Env(v),
            Capability::Clock => Kind::Clock(true),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<pb::Capability> for Capability {
    type Error = SandboxError;
    fn try_from(cap: pb::Capability) -> Result<Self, Self::Error> {
        use pb::capability::Kind;
        Ok(
            match cap.kind.ok_or_else(|| invalid("capability has no kind"))? {
                Kind::FsRead(v) => Capability::FsRead(v),
                Kind::FsWrite(v) => Capability::FsWrite(v),
                Kind::NetHttp(v) => Capability::NetHttp(v),
                Kind::NetTcp(v) => Capability::NetTcp(v),
                Kind::Exec(v) => Capability::Exec(v),
                Kind::Env(v) => Capability::Env(v),
                Kind::Clock(_) => Capability::Clock,
            },
        )
    }
}
