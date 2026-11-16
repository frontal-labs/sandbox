//! Integration tests for sandbox-policy.
//!
//! The capability and limit checks are pure functions over `NetworkPolicy` and
//! `ResourceLimits`, so they are tested directly without any network or OS
//! access. `resolve_and_check_host` is exercised only on the resolution-failure
//! path (a syntactically valid but unresolvable name), which avoids DNS flakiness.

use sandbox_policy::capability::*;
use sandbox_policy::limits::*;
use sandbox_policy::network::*;
use sandbox_types::policy::{Capability, NetworkPolicy, NetworkPolicyRule, ResourceLimits};
use sandbox_types::{ErrorCode, SandboxError};

// ---------------------------------------------------------------------------
// check_capability
// ---------------------------------------------------------------------------

#[test]
fn capability_exec_env_clock_fail_closed() {
    // The policy has no model for these capabilities yet, so they are denied,
    // even under an allow-all network policy.
    for policy in [NetworkPolicy::default(), NetworkPolicy::allow_all()] {
        assert!(!check_capability(&policy, &Capability::exec("python")));
        assert!(!check_capability(&policy, &Capability::env("PATH")));
        assert!(!check_capability(&policy, &Capability::clock()));
    }
}

#[test]
fn capability_net_http_follows_policy() {
    let deny = NetworkPolicy::default();
    assert!(!check_capability(
        &deny,
        &Capability::net_http("example.com")
    ));

    let allow_all = NetworkPolicy::allow_all();
    assert!(check_capability(
        &allow_all,
        &Capability::net_http("example.com")
    ));

    let github = create_github_policy();
    assert!(check_capability(
        &github,
        &Capability::net_http("github.com")
    ));
    assert!(!check_capability(
        &github,
        &Capability::net_http("example.com")
    ));
}

#[test]
fn capability_net_tcp_parses_host_port() {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_tcp("example.com", vec![443]));

    assert!(check_capability(
        &policy,
        &Capability::net_tcp("example.com:443")
    ));
    // Port not in the allowed set.
    assert!(!check_capability(
        &policy,
        &Capability::net_tcp("example.com:80")
    ));
    // Malformed address (no port) must be denied, not panic.
    assert!(!check_capability(
        &policy,
        &Capability::net_tcp("example.com")
    ));
    // Non-numeric port.
    assert!(!check_capability(
        &policy,
        &Capability::net_tcp("example.com:abc")
    ));
}

#[test]
fn capability_fs_checks_ignore_network_rules() {
    // A network host rule says nothing about filesystem paths. There is no path
    // policy yet, so filesystem checks are denied whatever host rules exist.
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("/workspace"));
    assert!(!check_fs_read(&policy, "/workspace"));
    assert!(!check_fs_write(&policy, "/workspace"));
    assert!(!check_fs_read(&policy, "/etc"));
    assert!(!check_fs_read(&NetworkPolicy::allow_all(), "/workspace"));
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
