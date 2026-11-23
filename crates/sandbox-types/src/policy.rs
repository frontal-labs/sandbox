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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
        self.decide(|r| r.matches_http(host))
    }

    pub fn check_tcp(&self, host: &str, port: u16) -> bool {
        self.decide(|r| r.matches_tcp(host, port))
    }

    /// A matching deny rule always wins, whatever the order of the rules and
    /// whether `default_deny` is set. Otherwise the request is allowed when
    /// `default_deny` is off or an allow rule matches.
    fn decide(&self, matches: impl Fn(&NetworkPolicyRule) -> bool) -> bool {
        let mut allowed = !self.default_deny;
        for rule in self.rules.iter().filter(|r| matches(r)) {
            match rule.action {
                PolicyAction::Deny => return false,
                PolicyAction::Allow => allowed = true,
            }
        }
        allowed
    }
}

/// True when `self`, a tenant's network policy, admits at least every (host,
/// port) pair that `other`, a session's network policy, admits.
///
/// The check compares rule shapes so it is independent of what a session might
/// later request:
///
/// - A default-open tenant (`self.default_deny` false) admits every (host,
///   port) its deny rules do not take. It covers a default-deny session
///   (which admits exactly its allow rules) unless a tenant deny rule takes
///   one of them. It covers a default-open session (which admits everything
///   its deny rules do not take) only when the tenant has no rules at all,
///   i.e. it is [`NetworkPolicy::allow_all`].
/// - A default-deny tenant admits only what its allow rules admit. It covers
///   a default-deny session when each session allow rule fits inside a tenant
///   allow rule with an equal-or-broader host set and, for TCP, whose ports
///   include the session's. It never covers a default-open session, because
///   the session admits everything its deny rules do not take, which is
///   always a superset of any finite allow set.
///
/// Deny rules in `other` only shrink what the session admits, so they never
/// fail the check. The comparison is conservative: a host pattern is admitted
/// to cover another only when it is `*`, the same pattern, or a wildcard whose
/// suffix the other's wildcard suffix is a subdomain of.
impl NetworkPolicy {
    pub fn is_superset_of(&self, other: &Self) -> bool {
        // A default-open tenant admits every (host, port) its deny rules do
        // not take. It covers a default-deny session (which admits exactly its
        // allow rules) unless a tenant deny rule takes one of them. It covers
        // a default-open session (which admits everything its deny rules do
        // not take) only when it has no rules at all, because any tenant rule
        // narrows the admitted set.
        if !self.default_deny {
            if other.default_deny {
                other.rules.iter().all(|rule| {
                    if rule.action != PolicyAction::Allow {
                        return true;
                    }
                    !self.rules.iter().any(|tenant_rule| {
                        tenant_rule.action == PolicyAction::Deny
                            && tenant_rule.host_pattern == rule.host_pattern
                    })
                })
            } else {
                self.rules.is_empty()
            }
        } else if other.default_deny {
            // Both default-deny: each session allow rule must fit inside a
            // tenant allow rule with an equal-or-broader host set and, for
            // TCP, whose ports include the session's.
            other.rules.iter().all(|rule| {
                if rule.action != PolicyAction::Allow {
                    return true;
                }
                self.rules.iter().any(|tenant_rule| {
                    tenant_rule.action == PolicyAction::Allow
                        && host_covers(&tenant_rule.host_pattern, &rule.host_pattern)
                        && match (&rule.ports, &tenant_rule.ports) {
                            (None, None) => true,
                            (Some(ports), Some(allowed)) => {
                                ports.iter().all(|p| allowed.contains(p))
                            }
                            (None, Some(_)) => false,
                            (Some(_), None) => false,
                        }
                })
            })
        } else {
            // A default-deny tenant admits only its allow rules; a default-
            // open session admits everything its deny rules do not take, which
            // is always a superset of any finite allow set. The tenant never
            // covers the session.
            false
        }
    }
}

/// True when a host matched by `inner` is always matched by `outer`, over the
/// pattern forms [`NetworkPolicyRule::matches_host`] understands: `*`, `*.suffix`
/// on a label boundary, or a literal. `*` covers everything. A literal covers
/// only the same literal. `*.suffix` covers `*.` followed by a subdomain of
/// `suffix` (including `suffix` itself as apex of the wildcard) and a literal
/// that is a subdomain of `suffix`.
fn host_covers(outer: &str, inner: &str) -> bool {
    let outer = outer.to_ascii_lowercase();
    let inner = inner.to_ascii_lowercase();
    if outer == "*" {
        return true;
    }
    let outer_suffix = outer.strip_prefix("*.");
    let inner_suffix = inner.strip_prefix("*.");
    match (outer_suffix, inner_suffix) {
        (Some(os), Some(is)) => is == os || is.ends_with(&format!(".{os}")),
        // A wildcard outer covers a literal inner when the inner is a
        // subdomain of the outer's suffix (on a label boundary).
        (Some(os), None) => {
            let os = os.trim_end_matches('.');
            !inner.is_empty() && inner.len() > os.len() && inner.ends_with(&format!(".{os}"))
        }
        // A literal outer covers a literal inner only when they are the same
        // host, compared case-insensitively and ignoring a trailing dot.
        (None, None) => normalize_host(&outer) == normalize_host(&inner),
        // A literal outer cannot cover a wildcard inner: the wildcard names
        // hosts the literal does not.
        (None, Some(_)) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetworkPolicyRule {
    pub action: PolicyAction,
    pub host_pattern: String,
    pub ports: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

    /// Host names compare case-insensitively and ignore a trailing dot.
    /// `*` matches any host. `*.example.com` matches subdomains of
    /// `example.com` on a label boundary: `a.example.com` matches, but
    /// `example.com` and `evilexample.com` do not. The apex needs its own rule.
    fn matches_host(&self, host: &str) -> bool {
        let host = normalize_host(host);
        if host.is_empty() {
            return false;
        }
        // The wildcard forms are recognised on the raw pattern. Trimming dots first
        // would turn `*.` into `*`, which matches every host.
        let raw = self.host_pattern.to_ascii_lowercase();
        if raw == "*" {
            return true;
        }
        if let Some(suffix) = raw.strip_prefix("*.") {
            let suffix = suffix.trim_end_matches('.');
            return !suffix.is_empty()
                && host.len() > suffix.len()
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
        }
        host == normalize_host(&raw)
    }
}

fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// A filesystem path policy. Like [`NetworkPolicy`], it is a default posture plus
/// a list of rules, and a matching deny rule always wins, whatever the order of
/// the rules and whether `default_deny` is set.
///
/// The rules name paths, not hosts. A `None` pattern is a catch-all that matches
/// every path; a `Some` pattern is a prefix, matched on a path-segment boundary
/// so `/work` matches `/work` and `/work/a/b` but not `/workdir`. Paths are
/// compared case-sensitively and without a trailing slash, and must be absolute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FsPolicy {
    pub default_deny: bool,
    pub rules: Vec<PathRule>,
}

impl Default for FsPolicy {
    fn default() -> Self {
        Self {
            default_deny: true,
            rules: Vec::new(),
        }
    }
}

impl FsPolicy {
    /// No rules and a deny posture: every path is denied.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// An allow posture with no rules: every path is allowed.
    pub fn allow_all() -> Self {
        Self {
            default_deny: false,
            rules: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: PathRule) {
        self.rules.push(rule);
    }

    /// Whether `path` is allowed for reads.
    pub fn allows_read(&self, path: &str) -> bool {
        self.decide(path)
    }

    /// Whether `path` is allowed for writes.
    pub fn allows_write(&self, path: &str) -> bool {
        self.decide(path)
    }

    /// A matching deny rule always wins, whatever the order of the rules and
    /// whether `default_deny` is set. Otherwise the path is allowed when
    /// `default_deny` is off or an allow rule matches.
    fn decide(&self, path: &str) -> bool {
        let mut allowed = !self.default_deny;
        for rule in self.rules.iter().filter(|r| r.matches(path)) {
            match rule.action {
                PolicyAction::Deny => return false,
                PolicyAction::Allow => allowed = true,
            }
        }
        allowed
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PathRule {
    pub action: PolicyAction,
    /// A path prefix to match, or `None` for a catch-all rule.
    pub path: Option<String>,
}

impl PathRule {
    pub fn allow(path: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Allow,
            path: Some(path.into()),
        }
    }

    pub fn deny(path: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Deny,
            path: Some(path.into()),
        }
    }

    /// A catch-all rule that matches every path.
    pub fn allow_all() -> Self {
        Self {
            action: PolicyAction::Allow,
            path: None,
        }
    }

    /// A catch-all rule that denies every path.
    pub fn deny_all() -> Self {
        Self {
            action: PolicyAction::Deny,
            path: None,
        }
    }

    /// A `None` pattern matches every path. A `Some` pattern matches when `path`
    /// is equal to it or lies beneath it on a segment boundary, so `/work` does
    /// not match `/workdir`. Relative and empty paths never match.
    fn matches(&self, path: &str) -> bool {
        let pattern = match &self.path {
            None => return true,
            Some(p) => p,
        };
        if path.is_empty() || !path.starts_with('/') {
            return false;
        }
        if pattern.is_empty() || !pattern.starts_with('/') {
            return false;
        }
        if path == pattern {
            return true;
        }
        let boundary = format!("{pattern}/");
        path.starts_with(&boundary)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretsConfig {
    pub secrets: HashMap<String, SecretEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretEntry {
    pub value: String,
    pub hosts: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(pattern: &str) -> NetworkPolicy {
        let mut policy = NetworkPolicy::default();
        policy.add_rule(NetworkPolicyRule::allow_http(pattern));
        policy
    }

    #[test]
    fn test_wildcard_matches_subdomains_on_a_label_boundary() {
        let policy = allow("*.example.com");
        assert!(policy.check_http("a.example.com"));
        assert!(policy.check_http("a.b.example.com"));
        assert!(!policy.check_http("evilexample.com"));
        assert!(!policy.check_http("example.com"));
        assert!(!policy.check_http("example.com.evil.org"));
    }

    #[test]
    fn test_empty_wildcard_suffix_matches_nothing() {
        let policy = allow("*.");
        assert!(!policy.check_http("example.com"));
        assert!(!policy.check_http("a."));
    }

    #[test]
    fn test_odd_patterns_do_not_panic() {
        // A pattern whose suffix starts with a multibyte character used to
        // slice inside it.
        for pattern in ["*.é.com", "*.日本.jp", "*.", "*", ""] {
            let policy = allow(pattern);
            let _ = policy.check_http("example.com");
            let _ = policy.check_http("");
            let _ = policy.check_tcp("é", 80);
        }
    }

    #[test]
    fn test_hosts_compare_case_insensitively_and_ignore_a_trailing_dot() {
        let policy = allow("Example.COM");
        assert!(policy.check_http("example.com"));
        assert!(policy.check_http("EXAMPLE.com."));
        assert!(!policy.check_http("example.org"));
        assert!(allow("*.Example.com").check_http("A.EXAMPLE.COM."));
    }

    #[test]
    fn test_empty_host_never_matches() {
        assert!(!allow("*").check_http(""));
        assert!(!allow("").check_http(""));
    }

    #[test]
    fn test_default_policy_denies_everything() {
        let policy = NetworkPolicy::default();
        assert!(!policy.check_http("example.com"));
        assert!(!policy.check_tcp("example.com", 443));
    }

    #[test]
    fn test_deny_rule_wins_over_allow_in_any_order() {
        let mut deny_last = NetworkPolicy::default();
        deny_last.add_rule(NetworkPolicyRule::allow_http("*"));
        deny_last.add_rule(NetworkPolicyRule::deny_http("evil.com"));
        assert!(!deny_last.check_http("evil.com"));
        assert!(deny_last.check_http("good.com"));

        let mut deny_first = NetworkPolicy::default();
        deny_first.add_rule(NetworkPolicyRule::deny_http("evil.com"));
        deny_first.add_rule(NetworkPolicyRule::allow_http("*"));
        assert!(!deny_first.check_http("evil.com"));
    }

    #[test]
    fn test_deny_rule_applies_when_default_deny_is_off() {
        let mut policy = NetworkPolicy::allow_all();
        policy.add_rule(NetworkPolicyRule::deny_http("evil.com"));
        assert!(!policy.check_http("evil.com"));
        assert!(policy.check_http("good.com"));
        assert!(NetworkPolicy::allow_all().check_tcp("any.host", 1));
    }

    #[test]
    fn test_tcp_rule_limits_ports() {
        let mut policy = NetworkPolicy::default();
        policy.add_rule(NetworkPolicyRule::allow_tcp("db.example.com", vec![5432]));
        assert!(policy.check_tcp("db.example.com", 5432));
        assert!(!policy.check_tcp("db.example.com", 22));
    }

    #[test]
    fn test_superset_default_open_session_needs_allow_all_tenant() {
        // A default-open session admits every host and port, so only an
        // allow_all tenant covers it. A tenant with any rule is too narrow.
        let open = NetworkPolicy::allow_all();
        assert!(NetworkPolicy::allow_all().is_superset_of(&open));
        let mut narrow = NetworkPolicy::allow_all();
        narrow.add_rule(NetworkPolicyRule::allow_http("api.example.com"));
        assert!(!narrow.is_superset_of(&open));
    }

    #[test]
    fn test_superset_default_open_tenant_covers_default_deny_session() {
        // A default-open tenant admits every (host, port) its deny rules do
        // not take, so it covers a default-deny session that names a host the
        // tenant does not deny.
        let session = allow("api.example.com");
        assert!(NetworkPolicy::allow_all().is_superset_of(&session));
        // A default-open tenant with a deny rule that takes the session's host
        // does not cover it.
        let mut tenant = NetworkPolicy::allow_all();
        tenant.add_rule(NetworkPolicyRule::deny_http("api.example.com"));
        assert!(!tenant.is_superset_of(&session));
        // A default-open tenant with an allow rule (which narrows nothing in
        // default-open mode) still covers the session.
        let mut tenant2 = NetworkPolicy::allow_all();
        tenant2.add_rule(NetworkPolicyRule::allow_http("api.example.com"));
        assert!(tenant2.is_superset_of(&session));
    }

    #[test]
    fn test_superset_default_deny_tenant_never_covers_default_open_session() {
        // A default-deny tenant admits only its allow rules; a default-open
        // session admits everything its deny rules do not take, which is
        // always a superset. The tenant never covers the session.
        let open = NetworkPolicy::allow_all();
        let tenant = allow("api.example.com");
        assert!(!tenant.is_superset_of(&open));
        let empty_tenant = NetworkPolicy::default();
        assert!(!empty_tenant.is_superset_of(&open));
    }

    #[test]
    fn test_superset_default_deny_session_fits_inside_tenant_rules() {
        let session = allow("api.example.com");
        // A wider wildcard covers the literal.
        let wider = allow("*.example.com");
        assert!(wider.is_superset_of(&session));
        // The same literal covers itself.
        assert!(allow("api.example.com").is_superset_of(&session));
        // A different literal does not.
        assert!(!allow("api.example.org").is_superset_of(&session));
        // A literal that is a subdomain of the session's wildcard does not
        // cover the wildcard (the wildcard names the apex's siblings too).
        assert!(!allow("api.example.com").is_superset_of(&allow("*.example.com")));
        // `*` covers everything, including a wildcard.
        assert!(allow("*").is_superset_of(&allow("*.example.com")));
        // A subdomain wildcard is covered by the broader wildcard.
        assert!(allow("*.example.com").is_superset_of(&allow("*.api.example.com")));
    }

    #[test]
    fn test_superset_port_lists_must_contain_session_ports() {
        let session_tcp = |ports: Vec<u16>| -> NetworkPolicy {
            let mut p = NetworkPolicy::default();
            p.add_rule(NetworkPolicyRule::allow_tcp("db.example.com", ports));
            p
        };
        let tenant_all_ports = |ports: Vec<u16>| -> NetworkPolicy {
            let mut p = NetworkPolicy::default();
            p.add_rule(NetworkPolicyRule::allow_tcp("db.example.com", ports));
            p
        };
        // The same ports cover each other.
        assert!(tenant_all_ports(vec![5432, 3306]).is_superset_of(&session_tcp(vec![5432])));
        // A tenant port list missing a session port does not cover it.
        assert!(!tenant_all_ports(vec![5432]).is_superset_of(&session_tcp(vec![5432, 3306])));
        // A tenant HTTP rule (no ports) does not cover a session TCP rule.
        assert!(!allow("db.example.com").is_superset_of(&session_tcp(vec![5432])));
        // A tenant TCP rule does not cover a session HTTP rule.
        assert!(!tenant_all_ports(vec![5432]).is_superset_of(&allow("db.example.com")));
    }

    #[test]
    fn test_superset_deny_rules_never_fail_the_check() {
        // The session denies a host the tenant allows; the session's deny
        // rule only shrinks what it admits, so the check still passes.
        let mut session = allow("*");
        session.default_deny = false;
        session.add_rule(NetworkPolicyRule::deny_http("evil.com"));
        assert!(NetworkPolicy::allow_all().is_superset_of(&session));
        // A default-deny session with a deny rule: the deny rule is skipped.
        let mut deny_session = NetworkPolicy::default();
        deny_session.add_rule(NetworkPolicyRule::allow_http("api.example.com"));
        deny_session.add_rule(NetworkPolicyRule::deny_http("evil.com"));
        assert!(allow("*.example.com").is_superset_of(&deny_session));
    }

    #[test]
    fn test_superset_empty_tenant_covers_only_default_deny_no_rules() {
        let empty_tenant = NetworkPolicy::default();
        let mut session = NetworkPolicy::default();
        session.add_rule(NetworkPolicyRule::allow_http("api.example.com"));
        // The tenant has no allow rule, so it covers nothing the session
        // admits.
        assert!(!empty_tenant.is_superset_of(&session));
        // But it covers a default-deny session that names no host at all.
        assert!(empty_tenant.is_superset_of(&NetworkPolicy::default()));
    }
}
