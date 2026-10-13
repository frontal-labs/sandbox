# sandbox-policy

Policy helpers for capability checks, network allowlists, and resource limits.

**Status: partial.** The check functions and preset network policies exist and compile in the workspace. The policy types they use live in `sandbox-types`. There is no `PolicyEngine`, no per-session grant handling, no trace emission, no secrets injection, and no DNS pinning. Filesystem checks currently reuse the HTTP host check (see below). No tests exist yet.

## Public API

Re-exported from `src/lib.rs`:

- `capability.rs`
  - `check_capability(&NetworkPolicy, &Capability) -> bool`
  - `check_fs_read`, `check_fs_write`, `check_net_http`, `check_net_tcp(policy, host, port)`
  - `resolve_and_check_host(host, &NetworkPolicy) -> Result<IpAddr, SandboxError>`: resolves the host, rejects loopback, private, link-local, broadcast, documentation, and unique-local addresses, then applies the HTTP allowlist.
- `limits.rs`: `check_memory_limit`, `check_cpu_limit`, `check_file_size`, `check_total_size`, `check_inodes`, `check_path_depth`. Each takes `&ResourceLimits` and a requested value, and returns `resource_exhausted` on overflow. Each is a no-op when its limit is `None`.
- `network.rs`: `create_default_policy`, `create_allow_all_policy`, and preset allowlists `create_github_policy`, `create_npm_policy`, `create_pypi_policy`, `create_docker_policy`.

The types `Capability`, `NetworkPolicy`, `NetworkPolicyRule`, and `ResourceLimits` come from `sandbox-types::policy`.

## Behaviour

- `NetworkPolicy::default()` has `default_deny: true` and no rules, so all egress is denied. `allow_all()` disables default deny.
- `Capability::Exec`, `Env`, and `Clock` always return `true`. They are not enforced.
- `check_fs_read` and `check_fs_write` call `check_http` on the path string. This is a placeholder and must not be relied on for file access. Filesystem access control belongs in `sandbox-vfs` or a dedicated policy type.

## Invariants

- Network access is default-deny unless a rule allows the host.
- Private and link-local IP ranges are rejected by `resolve_and_check_host`.

## Security assumptions and gaps

- `resolve_and_check_host` returns the first resolved address, but callers must connect to that address for DNS pinning to hold. Nothing enforces this yet.
- Private IP checks cover IPv4 loopback/private/link-local and IPv6 loopback/link-local/unique-local. The metadata IP is covered only if it falls in link-local `169.254.0.0/16`.
- Exec, env, and clock capabilities are unconditionally allowed.

## Example

```rust
use sandbox_policy::{check_capability, create_github_policy};
use sandbox_types::policy::Capability;

let policy = create_github_policy();
let allowed = check_capability(&policy, &Capability::net_http("api.github.com"));
```

Not compiled in this pass. Checked against `src/capability.rs`, `src/network.rs`, and `sandbox-types/src/policy.rs`.

## Spec

See [PLAN.md](../../PLAN.md) section 4.7. Planned: per-session capability sets, trace events on deny, secrets injection on egress, and a shared `ResourceLimits` with the fields in the PLAN.
