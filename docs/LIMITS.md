# Limits and Quotas

This page lists the resource limits defined in code today, where they live, and whether anything enforces them yet. Target performance numbers are in [PERFORMANCE.md](PERFORMANCE.md) and PLAN.md section 6. Quota tiers, billing, and scheduler metering are planned and are not described with numbers here.

## Status summary

| Area | Defined in code | Enforced in code |
|------|-----------------|------------------|
| Session resource limits (`ResourceLimits`) | Yes | No. The `check_*` helpers in `crates/sandbox-policy/src/limits.rs` have no callers in the repo. |
| VFS quotas (`Quotas`) | Yes | Yes, on create and write: `total_bytes`, `max_file_size`, `max_path_depth`, and `inode_count`. Checks are not reserved, so concurrent writers can overshoot. |
| Network policy defaults | Yes | Partly. Policy helpers exist in `crates/sandbox-policy/src/network.rs`; nothing on the exec or fetch path calls them yet. See [SECURITY_MODEL.md](SECURITY_MODEL.md). |
| API key quotas and tiers | No | No |
| Process, network, and hard limits from earlier drafts | No | No |

Treat every number below as a default value, not a guarantee.

## Session resource limits

Source: `ResourceLimits::default()` in `crates/sandbox-types/src/policy.rs`. Every field is `Option<_>`, so `None` means "no limit" for that field.

| Field | Default | Unit | Notes |
|-------|---------|------|-------|
| `memory_bytes` | 512 MiB (536,870,912) | bytes | Memory cap for a session |
| `cpu_ms_per_sec` | 1000 | ms | Field name indicates a per-second CPU budget. No enforcement code exists, so the exact semantics are not yet defined. |
| `wall_timeout_secs` | 300 | s | Wall-clock limit |
| `max_file_size` | 100 MiB | bytes | Per file |
| `max_total_size` | 1 GiB | bytes | Total VFS bytes |
| `max_inodes` | 100,000 | count | Files plus directories |
| `max_path_depth` | 256 | count | Directory nesting depth |
| `max_processes` | 100 | count | Virtual process count |
| `max_open_files` | 1024 | count | Open file handles |

Per-exec timeout is separate: `ExecRequest.timeout_ms` in `crates/sandbox-types/src/session.rs` is optional and has no default.

Sessions take their limits from `SessionConfig.limits` (`crates/sandbox-types/src/session.rs`). `SessionConfig::default()` uses `ResourceLimits::default()`.

## VFS quotas

Source: `Quotas::default()` in `crates/sandbox-vfs/src/quota.rs`.

| Field | Default |
|-------|---------|
| `total_bytes` | 1 GiB |
| `inode_count` | 100,000 |
| `max_file_size` | 100 MiB |
| `max_path_depth` | 256 |

These duplicate the session defaults above. All four fields are enforced (see status summary). Keep the two sets in sync until one source is used.

## Network defaults

Source: `crates/sandbox-types/src/policy.rs`.

- `NetworkPolicy::default()` sets `default_deny: true` with no rules. All egress is denied until a rule is added.
- `NetworkPolicy::allow_all()` disables the default-deny check. Use it only for tests.
- Rules match by host pattern: `*` matches any host, `*.suffix` matches by suffix, and anything else matches exactly. Port lists are optional for TCP rules.

Request and response size caps, connection counts, and request timeouts are not defined in code yet. The HTTP client that would enforce them is planned (PLAN.md section 4.7).

## Error reporting

Limit violations are reported with `ErrorCode::ResourceExhausted` (`crates/sandbox-types/src/error.rs`). Policy denials use `ErrorCode::PolicyDenied`. The HTTP status mapping for these codes is not implemented: `crates/sandbox-server` has no handler that maps them yet, and the handlers are placeholders. Trace events for denials (`PolicyDenied`) are defined in `crates/sandbox-types/src/event.rs`, but nothing emits them yet.

## Planned limits

These appear in PLAN.md and are not defined in code:

- Scheduler quotas per API key, tier-based session and CPU budgets, and metering (PLAN.md sections 4.14 and 4.15).
- Hard, non-configurable ceilings on session lifetime, snapshot depth, fork count, and trace size. No values are fixed in code or in PLAN.md.
- Capacity targets for a single host (PLAN.md section 6, for example "10,000+ idle sessions per 64 GB host").

Add concrete values to this file when the matching code exists, and cite the source line.

## Worker channel

These limits apply to the gRPC channel between the scheduler and a worker
(`crates/sandbox-worker`, `crates/sandbox-proto`):

| Limit | Value | Enforced by |
|-------|-------|-------------|
| Request or response message | 16 MiB (`MAX_MESSAGE_BYTES`) | Host check before sending, and tonic decoding limits on both sides |
| Script size | 16 MiB | Host check (`ResourceExhausted`) |
| Call deadline | 60 s (`RPC_DEADLINE`) | Host sets the deadline on each call |
| Length-prefixed guest frame | 16 MiB (`framing::MAX_FRAME_BYTES`) | Checked before allocation (no caller yet; VM tier) |
