//! Integration tests for sandbox-policy.
//!
//! The capability and limit checks are pure functions over `NetworkPolicy`,
//! `FsPolicy`, and `ResourceLimits`, so they are tested directly without any
//! network or OS access. `resolve_and_check_host` is exercised only on the
//! resolution-failure path (a syntactically valid but unresolvable name),
//! which avoids DNS flakiness.

use sandbox_policy::capability::*;
use sandbox_policy::limits::*;
use sandbox_policy::network::*;
use sandbox_types::policy::{
    Capability, FsPolicy, NetworkPolicy, NetworkPolicyRule, PathRule, ResourceLimits,
};
use sandbox_types::{ErrorCode, SandboxError};

/// Filesystem axis for network-focused capability tests: an explicit allow-all
/// policy keeps the filesystem out of the way.
fn fs_allow_all() -> FsPolicy {
    FsPolicy::allow_all()
}

// ---------------------------------------------------------------------------
// FsPolicy
// ---------------------------------------------------------------------------

#[test]
fn fs_policy_default_denies_everything() {
    let fs = FsPolicy::default();
    assert!(fs.default_deny);
    assert!(fs.rules.is_empty());
    assert!(!fs.allows_read("/work/a.txt"));
    assert!(!fs.allows_write("/work/a.txt"));
}

#[test]
fn fs_policy_allow_all_permits_everything() {
    let fs = FsPolicy::allow_all();
    assert!(!fs.default_deny);
    assert!(fs.allows_read("/etc/hostname"));
    assert!(fs.allows_write("/etc/hostname"));
}

#[test]
fn fs_policy_path_rules_match_on_segment_boundaries() {
    let mut fs = FsPolicy::deny_all();
    fs.add_rule(PathRule::allow("/work"));

    // The rule root and everything below it match.
    assert!(fs.allows_read("/work"));
    assert!(fs.allows_write("/work/a.txt"));
    assert!(fs.allows_read("/work/deep/nested/file"));

    // A sibling that only shares a prefix must NOT match.
    assert!(!fs.allows_read("/workdir"));
    assert!(!fs.allows_write("/workdir/a.txt"));

    // Nothing outside the rule matches under deny_all.
    assert!(!fs.allows_read("/tmp"));
    assert!(!fs.allows_write("/etc"));
}

#[test]
fn fs_policy_deny_rules_win_over_allow_rules() {
    let mut fs = FsPolicy::deny_all();
    fs.add_rule(PathRule::allow("/work"));
    fs.add_rule(PathRule::deny("/work/secret"));

    assert!(fs.allows_read("/work/a.txt"));
    assert!(!fs.allows_read("/work/secret/key.txt"));
    assert!(!fs.allows_write("/work/secret"));

    // Deny wins regardless of rule order.
    let mut reordered = FsPolicy::deny_all();
    reordered.add_rule(PathRule::deny("/work/secret"));
    reordered.add_rule(PathRule::allow("/work"));
    assert!(!reordered.allows_read("/work/secret/key.txt"));
    assert!(reordered.allows_read("/work/a.txt"));
}

#[test]
fn fs_policy_allow_all_policy_honors_deny_rules() {
    let mut fs = FsPolicy::allow_all();
    fs.add_rule(PathRule::deny("/etc"));

    assert!(fs.allows_read("/work/a.txt"));
    assert!(!fs.allows_read("/etc/passwd"));
    assert!(!fs.allows_write("/etc/passwd"));
}

#[test]
fn fs_policy_catch_all_rules_match_every_path() {
    let deny_everything = FsPolicy {
        default_deny: true,
        rules: vec![PathRule::deny_all()],
    };
    assert!(!deny_everything.allows_read("/anything/at/all"));

    let allow_everything = FsPolicy {
        default_deny: true,
        rules: vec![PathRule::allow_all()],
    };
    assert!(allow_everything.allows_read("/anything/at/all"));
    assert!(allow_everything.allows_write("/anything/at/all"));
}

#[test]
fn fs_policy_empty_and_relative_paths_never_match() {
    let mut fs = FsPolicy::deny_all();
    fs.add_rule(PathRule::allow(""));
    fs.add_rule(PathRule::allow("relative/path"));

    assert!(!fs.allows_read(""));
    assert!(!fs.allows_read("/"));
    assert!(!fs.allows_read("relative/path"));
    assert!(!fs.allows_write("/work"));
}

#[test]
fn fs_policy_read_and_write_follow_the_same_rules() {
    // FsPolicy has a single rule set; a read that is denied is a write that is
    // denied too. Quotas are a separate axis owned by the VFS.
    let mut fs = FsPolicy::deny_all();
    fs.add_rule(PathRule::allow("/work"));

    assert_eq!(fs.allows_read("/work/x"), fs.allows_write("/work/x"));
    assert_eq!(fs.allows_read("/elsewhere"), fs.allows_write("/elsewhere"));
}

// ---------------------------------------------------------------------------
// check_capability
// ---------------------------------------------------------------------------

#[test]
fn capability_exec_env_clock_fail_closed() {
    // The policy has no model for these capabilities, so they are denied,
    // even under an allow-all network and filesystem policy.
    for network in [NetworkPolicy::default(), NetworkPolicy::allow_all()] {
        for fs in [FsPolicy::default(), FsPolicy::allow_all()] {
            assert!(!check_capability(
                &network,
                &fs,
                &Capability::exec("python")
            ));
            assert!(!check_capability(&network, &fs, &Capability::env("PATH")));
            assert!(!check_capability(&network, &fs, &Capability::clock()));
        }
    }
}

#[test]
fn capability_net_http_follows_policy() {
    let deny = NetworkPolicy::default();
    assert!(!check_capability(
        &deny,
        &fs_allow_all(),
        &Capability::net_http("example.com")
    ));

    let allow_all = NetworkPolicy::allow_all();
    assert!(check_capability(
        &allow_all,
        &fs_allow_all(),
        &Capability::net_http("example.com")
    ));

    let github = create_github_policy();
    assert!(check_capability(
        &github,
        &fs_allow_all(),
        &Capability::net_http("github.com")
    ));
    assert!(!check_capability(
        &github,
        &fs_allow_all(),
        &Capability::net_http("example.com")
    ));
}

#[test]
fn capability_net_tcp_parses_host_port() {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_tcp("example.com", vec![443]));
    let fs = fs_allow_all();

    assert!(check_capability(
        &policy,
        &fs,
        &Capability::net_tcp("example.com:443")
    ));
    // Port not in the allowed set.
    assert!(!check_capability(
        &policy,
        &fs,
        &Capability::net_tcp("example.com:80")
    ));
    // Malformed address (no port) must be denied, not panic.
    assert!(!check_capability(
        &policy,
        &fs,
        &Capability::net_tcp("example.com")
    ));
    // Non-numeric port.
    assert!(!check_capability(
        &policy,
        &fs,
        &Capability::net_tcp("example.com:abc")
    ));
}

#[test]
fn capability_fs_checks_follow_the_fs_policy() {
    // Network host rules say nothing about filesystem paths, and vice versa:
    // each capability axis is checked against its own policy.
    let mut network = NetworkPolicy::default();
    network.add_rule(NetworkPolicyRule::allow_http("/workspace"));

    let mut fs = FsPolicy::deny_all();
    fs.add_rule(PathRule::allow("/workspace"));
    fs.add_rule(PathRule::deny("/workspace/secret"));

    assert!(check_capability(
        &network,
        &fs,
        &Capability::fs_read("/workspace/a.txt")
    ));
    assert!(!check_capability(
        &network,
        &fs,
        &Capability::fs_read("/workspace/secret/key")
    ));
    assert!(!check_capability(
        &network,
        &fs,
        &Capability::fs_read("/etc")
    ));
    assert!(!check_capability(
        &NetworkPolicy::allow_all(),
        &FsPolicy::default(),
        &Capability::fs_read("/workspace")
    ));
}

// ---------------------------------------------------------------------------
// limits
// ---------------------------------------------------------------------------

fn assert_exhausted(e: &SandboxError) {
    assert!(
        e.code == ErrorCode::ResourceExhausted,
        "expected ResourceExhausted, got {e:?}"
    );
}

#[test]
fn memory_limit() {
    let limits = ResourceLimits {
        memory_bytes: Some(1024),
        ..ResourceLimits::default()
    };
    assert!(check_memory_limit(&limits, 1024).is_ok()); // exactly at limit
    assert_exhausted(&check_memory_limit(&limits, 1025).unwrap_err());

    // No limit configured: always ok.
    let unlimited = ResourceLimits {
        memory_bytes: None,
        ..ResourceLimits::default()
    };
    assert!(check_memory_limit(&unlimited, u64::MAX).is_ok());
}

#[test]
fn cpu_limit() {
    let limits = ResourceLimits {
        cpu_ms_per_sec: Some(500),
        ..ResourceLimits::default()
    };
    assert!(check_cpu_limit(&limits, 500).is_ok());
    assert_exhausted(&check_cpu_limit(&limits, 501).unwrap_err());
}

#[test]
fn file_size_limit() {
    let limits = ResourceLimits {
        max_file_size: Some(100),
        ..ResourceLimits::default()
    };
    assert!(check_file_size(&limits, 100).is_ok());
    assert_exhausted(&check_file_size(&limits, 101).unwrap_err());
}

#[test]
fn total_size_limit() {
    let limits = ResourceLimits {
        max_total_size: Some(1000),
        ..ResourceLimits::default()
    };
    assert!(check_total_size(&limits, 1000).is_ok());
    assert_exhausted(&check_total_size(&limits, 1001).unwrap_err());
}

#[test]
fn inode_limit() {
    let limits = ResourceLimits {
        max_inodes: Some(1000),
        ..ResourceLimits::default()
    };
    assert!(check_inodes(&limits, 1000).is_ok());
    assert_exhausted(&check_inodes(&limits, 1001).unwrap_err());
}

#[test]
fn path_depth_limit() {
    let limits = ResourceLimits {
        max_path_depth: Some(10),
        ..ResourceLimits::default()
    };
    assert!(check_path_depth(&limits, 10).is_ok());
    assert_exhausted(&check_path_depth(&limits, 11).unwrap_err());
}

// ---------------------------------------------------------------------------
// network policy helpers
// ---------------------------------------------------------------------------

#[test]
fn default_policy_denies_everything() {
    let policy = create_default_policy();
    assert!(policy.default_deny);
    assert!(!policy.check_http("example.com"));
    assert!(!policy.check_tcp("example.com", 443));
}

#[test]
fn allow_all_policy_permits_everything() {
    let policy = create_allow_all_policy();
    assert!(!policy.default_deny);
    assert!(policy.check_http("example.com"));
    assert!(policy.check_tcp("example.com", 443));
}

#[test]
fn curated_policies_allow_only_their_hosts() {
    let github = create_github_policy();
    assert!(github.check_http("github.com"));
    assert!(github.check_http("api.github.com"));
    assert!(github.check_http("raw.githubusercontent.com"));
    assert!(github.check_http("objects.githubusercontent.com"));
    assert!(!github.check_http("example.com"));

    let npm = create_npm_policy();
    assert!(npm.check_http("registry.npmjs.org"));
    assert!(npm.check_http("api.npmjs.org"));
    assert!(!npm.check_http("pypi.org"));

    let pypi = create_pypi_policy();
    assert!(pypi.check_http("pypi.org"));
    assert!(pypi.check_http("files.pythonhosted.org"));
    assert!(!pypi.check_http("example.com"));

    let docker = create_docker_policy();
    assert!(docker.check_http("registry-1.docker.io"));
    assert!(docker.check_http("auth.docker.io"));
    assert!(!docker.check_http("example.com"));
}

#[test]
fn deny_rule_does_not_grant_access() {
    // A Deny rule that matches a host must not satisfy check_http, even when it
    // is the only rule.
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::deny_http("example.com"));
    assert!(!policy.check_http("example.com"));
}

#[test]
fn wildcard_rules_match_any_host() {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("*"));
    assert!(policy.check_http("example.com"));
    assert!(policy.check_http("anything.else"));
}

// ---------------------------------------------------------------------------
// resolve_and_check_host (resolution-failure path only)
// ---------------------------------------------------------------------------

#[test]
fn unresolvable_host_is_a_network_error() {
    // A name that cannot resolve to any address: the function must report a
    // network error rather than panicking or returning a success.
    let policy = NetworkPolicy::allow_all();
    let err = resolve_and_check_host("this-host-does-not-exist.invalid", &policy)
        .expect_err("unresolvable host must fail");
    assert!(
        err.code == ErrorCode::NetworkError,
        "expected NetworkError, got {err:?}"
    );
}
