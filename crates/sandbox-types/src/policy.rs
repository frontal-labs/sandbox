use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    FsRead(String),
    FsWrite(String),
    NetHttp(String),
    NetTcp(String),
    Exec(String),
    Env(String),
    Clock,
}

impl Capability {
    pub fn fs_read(path: impl Into<String>) -> Self {
        Self::FsRead(path.into())
    }

    pub fn fs_write(path: impl Into<String>) -> Self {
        Self::FsWrite(path.into())
    }

    pub fn net_http(host_pattern: impl Into<String>) -> Self {
        Self::NetHttp(host_pattern.into())
    }

    pub fn net_tcp(host_port: impl Into<String>) -> Self {
        Self::NetTcp(host_port.into())
    }

    pub fn exec(tool: impl Into<String>) -> Self {
        Self::Exec(tool.into())
    }

    pub fn env(name: impl Into<String>) -> Self {
        Self::Env(name.into())
    }

    pub fn clock() -> Self {
        Self::Clock
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub default_deny: bool,
    pub rules: Vec<NetworkPolicyRule>,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            default_deny: true,
            rules: Vec::new(),
        }
    }
}

impl NetworkPolicy {
    pub fn allow_all() -> Self {
        Self {
            default_deny: false,
            rules: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: NetworkPolicyRule) {
        self.rules.push(rule);
    }

    pub fn check_http(&self, host: &str) -> bool {
        if !self.default_deny {
            return true;
        }
        self.rules
            .iter()
            .any(|r| r.matches_http(host) && r.action == PolicyAction::Allow)
    }

    pub fn check_tcp(&self, host: &str, port: u16) -> bool {
        if !self.default_deny {
            return true;
        }
        self.rules
            .iter()
            .any(|r| r.matches_tcp(host, port) && r.action == PolicyAction::Allow)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicyRule {
    pub action: PolicyAction,
    pub host_pattern: String,
    pub ports: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    Allow,
    Deny,
}

impl NetworkPolicyRule {
    pub fn allow_http(host_pattern: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Allow,
            host_pattern: host_pattern.into(),
            ports: None,
        }
    }

    pub fn allow_tcp(host_pattern: impl Into<String>, ports: Vec<u16>) -> Self {
        Self {
            action: PolicyAction::Allow,
            host_pattern: host_pattern.into(),
            ports: Some(ports),
        }
    }

    pub fn deny_http(host_pattern: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Deny,
            host_pattern: host_pattern.into(),
            ports: None,
        }
    }

    pub fn matches_http(&self, host: &str) -> bool {
        self.matches_host(host)
    }

    pub fn matches_tcp(&self, host: &str, port: u16) -> bool {
        self.matches_host(host) && self.ports.as_ref().is_none_or(|p| p.contains(&port))
    }

    fn matches_host(&self, host: &str) -> bool {
        if self.host_pattern == "*" {
            return true;
        }
        if let Some(suffix) = self.host_pattern.strip_prefix("*.") {
            return host.ends_with(suffix) || host == &suffix[1..];
        }
        host == self.host_pattern
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_bytes: Option<u64>,
    pub cpu_ms_per_sec: Option<u64>,
    pub wall_timeout_secs: Option<u64>,
    pub max_file_size: Option<u64>,
    pub max_total_size: Option<u64>,
    pub max_inodes: Option<u64>,
    pub max_path_depth: Option<u32>,
    pub max_processes: Option<u32>,
    pub max_open_files: Option<u32>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: Some(512 * 1024 * 1024),
            cpu_ms_per_sec: Some(1000),
            wall_timeout_secs: Some(300),
            max_file_size: Some(100 * 1024 * 1024),
            max_total_size: Some(1024 * 1024 * 1024),
            max_inodes: Some(100000),
            max_path_depth: Some(256),
            max_processes: Some(100),
            max_open_files: Some(1024),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretsConfig {
    pub secrets: HashMap<String, SecretEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretEntry {
    pub value: String,
    pub hosts: Vec<String>,
}
