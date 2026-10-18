# Sandbox Types

Shared types for the sandbox system.

## Status

Implemented (data types and serde derives). No sandbox crate dependencies. `JsonSchema` derives are not present yet, even though `schemars` is a dependency, so OpenAPI generation is planned, not working.

## Overview

Fundamental IDs, error codes, trace events, session configuration, and policy types used across the other crates. Source: `crates/sandbox-types/src/`.

## Public API (from `src/lib.rs`)

- **IDs** (`ids.rs`): `SessionId`, `SnapshotId`, `TenantId` (ULID newtypes; `new`, `from_ulid`, `as_ulid`).
- **Hash** (`hash.rs`): `ContentHash` (BLAKE3, 32 bytes; `new`, `from_hash`, `as_hash`, `as_bytes`, `to_hex`, `from_hex`, `nil`).
- **Errors** (`error.rs`): `ErrorCode` (serializable enum) and `SandboxError` (`thiserror`; constructors such as `new`, `invalid_argument`, `not_found`, `policy_denied`, `unsupported`). `SandboxError` itself is not serializable.
- **Events** (`event.rs`): `Event` with variants `SessionCreated`, `ExecStarted`, `ExecExited`, `StdoutChunk`, `StderrChunk`, `FileChanged`, `NetRequest`, `PolicyDenied`, `SnapshotTaken`, `Promoted`, `Slept`, `Woken`, `SessionDestroyed`; `Event::session_id()`; `FileOp`.
- **Session** (`session.rs`): `SessionConfig` (`new(tenant_id)` plus `with_tier`, `with_limits`, `with_network`, `with_env`, `with_cwd`, `with_snapshot`, `with_secrets`, `with_label`), `SessionState`, `SessionStatus`, `Tier`, `TierPreference` (`Auto` default, `Isolate`, `Vm`), `ExecRequest`, `ExecResult`, `Snapshot`.
- **Policy** (`policy.rs`): `Capability`, `NetworkPolicy` (`allow_all`, `add_rule`, `check_http`, `check_tcp`), `NetworkPolicyRule` and `PolicyAction`, `ResourceLimits`, `SecretsConfig`, `SecretEntry`.

## Invariants and security assumptions

- `NetworkPolicy::check_*` returns `true` unless `default_deny` is set. Deny-by-default must be set explicitly by the caller.
- `SecretEntry.value` is a plain `String` and the type derives `Debug`. Secret values can leak through debug output or logs; callers must not log `SessionConfig` or `SecretsConfig` as-is.
- `ContentHash` is the content address used by the CAS and VFS; equal hashes are assumed to mean equal content.

## Example

```rust
use sandbox_types::{SessionConfig, TenantId, TierPreference};

let config = SessionConfig::new(TenantId::new()).with_tier(TierPreference::Auto);
```

See `plans/PLAN.md` section 4.1 for the target spec.
