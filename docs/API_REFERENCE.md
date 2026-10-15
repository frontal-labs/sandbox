# API Reference

> **Status: early.** Session, exec, filesystem, snapshot, and fork routes work
> against in-memory sessions, and every `/v1` route requires a valid API key.
> Settings the backend does not enforce (VM tier, custom limits, network policy,
> secrets, environment, working directory) are refused with `501`, not ignored. The full target API is specified in
> [PLAN.md section 4.15](../PLAN.md#415-sandbox-server).

## Base URL

The intended bind address is `0.0.0.0:8080` (the default of `sandbox serve
--addr` in `cli/src/commands.rs`), but no process currently serves these
routes (see the CLI section). Routes are mounted under `/v1`.

```
http://localhost:8080/v1
```

The SDKs default to `SANDBOX_BASE_URL=http://localhost:8080`. There is no
hosted endpoint yet.

**Wire format:** all SDKs use the server's snake_case field names. Timestamps are
RFC 3339 strings, and byte fields (stdin, stdout, stderr) are arrays of 0-255
integers. The SDKs convert those arrays to bytes on their side.

## Routed endpoints

| Method | Path | Handler | Behavior |
|--------|------|-----------------------|-------|
| POST | `/v1/sessions` | `create_session` | Create a session |
| GET | `/v1/sessions/{id}` | `get_session` | Status, tier |
| DELETE | `/v1/sessions/{id}` | `destroy_session` | Destroy a session |
| POST | `/v1/sessions/{id}/exec` | `exec_command` | Run a command |
| GET | `/v1/sessions/{id}/fs/{path}` | `read_file` | Read a file |
| PUT | `/v1/sessions/{id}/fs/{path}` | `write_file` | Write a file |
| DELETE | `/v1/sessions/{id}/fs/{path}` | `delete_file` | Delete a file |
| POST | `/v1/sessions/{id}/snapshot` | `take_snapshot` | Snapshot the committed filesystem |
| POST | `/v1/snapshots/{id}/fork` | `fork_snapshot` | Start 1 to 64 sessions from a snapshot |
| GET | `/v1/sessions/{id}/snapshots` | `list_snapshots` | Snapshots taken from a session |
| GET | `/v1/sessions/{id}/snapshots/{snapshot_id}` | `get_snapshot` | One snapshot taken from a session |

Source: `crates/sandbox-server/src/api.rs` (`create_router`).

## Contract files

- REST: `openapi/v1/openapi.json` and `openapi/v1/openapi.yaml`, generated from `crates/sandbox-server/src/api.rs`. Every route and its responses are listed there.
- Internal and gRPC: `proto/sandbox/v1/`, compiled into `crates/sandbox-proto`. The worker service is live. The `api` services are schema only.

## Planned, not routed

These appear in PLAN.md section 4.15 but have no route in `api.rs`:

- `GET /v1/sessions/{id}/ws`: interactive WebSocket
- `GET /v1/sessions/{id}/fs?list=...`: directory listing
- `GET /v1/sessions/{id}/trace`: trace event stream
- `GET /v1/usage`: metering
- Served now: `GET /v1/openapi.json` returns the generated spec (no auth). The
  committed copies are in `openapi/v1/`, and `openapi/README.md` covers
  regeneration and the drift check.

## Authentication

Every `/v1` route requires `Authorization: Bearer <key>`. Missing or invalid
keys get `401 Unauthorized`.

Keys have the form `sbx_<key-id>_<secret>`. `AuthManager::create_key` returns
the plaintext once. The store keeps only a BLAKE3 digest of the secret, and
`revoke` removes a key by id. Per-key scopes and quotas are still planned
(PLAN.md section 4.15).

## Error format

The error vocabulary exists in `crates/sandbox-types/src/error.rs` as the
`ErrorCode` enum (for example `NotFound`, `InvalidArgument`, `PolicyDenied`,
`Timeout`, `Unsupported`) with a `SandboxError` type. No HTTP mapping from
these codes to status codes is implemented yet. The HTTP error envelope below
is a proposal:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Human-readable description",
    "details": {}
  }
}
```

## Shared types (crate `sandbox-types`)

These are the Rust types the API will serialize. Field names are taken from
the source. JSON encoding is not yet exercised by any handler.

- **`Tier`** (`session.rs`): `Isolate`, `Vm`. Serialized lowercase
  (`"isolate"`, `"vm"`).
- **`TierPreference`** (`session.rs`): `Auto` (default), `Isolate`, `Vm`.
- **`SessionStatus`** (`session.rs`): `Creating`, `Running`, `Sleeping`,
  `Promoting`, `Destroyed`, `Error`.
- **`SessionConfig`** (`session.rs`): `tenant_id`, `tier_preference`, `limits`,
  `network`, `env`, `cwd`, `initial_snapshot`, `secrets`, `labels`.
- **`ExecRequest`** (`session.rs`): `argv`, `env`, `cwd`, `stdin`, `timeout_ms`,
  `stream`.
- **`ExecResult`** (`session.rs`): `exit_code`, `stdout`, `stderr`, `cpu_ms`,
  `wall_ms`.
- **`Snapshot`** (`session.rs`): `id`, `parent`, `vfs_root`, `env`, `cwd`,
  `tier`, `runtime_blob`, `created_at`, `labels`.
- **`Event`** (`event.rs`): `SessionCreated`, `ExecStarted`, `ExecExited`,
  `StdoutChunk`, `StderrChunk`, `FileChanged`, `NetRequest`, `PolicyDenied`,
  `SnapshotTaken`, `Promoted`, `Slept`, `Woken`, `SessionDestroyed`.
- **`ResourceLimits`, `NetworkPolicy`, `SecretsConfig`** (`policy.rs`).

## Target request and response shapes

The examples below show the intended JSON for each route. They are design
targets, drawn from the earlier draft of this document and aligned with the
types above where possible. They are not implemented.

### Create session (planned)

```
POST /v1/sessions
```

```json
{
  "tier": "auto",
  "limits": { "memory_mb": 512, "cpu_ms": 10000, "wall_timeout_ms": 30000 },
  "network": { "allow": ["api.github.com", "*.npmjs.org"] },
  "env": { "NODE_ENV": "production" },
  "cwd": "/work",
  "initial_snapshot": "snap_01HX..."
}
```

Planned response: `201` with `{"id": "sess_...", "tier": "isolate", "status": "running", ...}`.
Field names under `limits` and `network` must be confirmed against
`ResourceLimits` and `NetworkPolicy` in `policy.rs` before implementation.

### Get session (planned)

```
GET /v1/sessions/{id}
```

Planned response: `200` with `id`, `tier`, `status`, `created_at`, and usage
counters. Usage reporting is not yet defined in the types.

### Destroy session (planned)

```
DELETE /v1/sessions/{id}
```

Planned response: `204`.

### Execute command (planned)

```
POST /v1/sessions/{id}/exec
```

```json
{
  "argv": ["npm", "test"],
  "cwd": "/work",
  "env": { "CI": "true" },
  "stdin": "base64_encoded_input",
  "timeout_ms": 60000
}
```

Planned streaming response (`?stream=true`, NDJSON):

```json
{"type": "stdout", "data": "base64_chunk"}
{"type": "stderr", "data": "base64_chunk"}
{"type": "exit", "code": 0, "cpu_ms": 500, "wall_ms": 1200}
```

The SDKs already call `POST .../exec?stream=true`. The server ignores the
query string today.

### File read, write, delete (planned)

```
GET    /v1/sessions/{id}/fs/{path}
PUT    /v1/sessions/{id}/fs/{path}
DELETE /v1/sessions/{id}/fs/{path}
```

Write body (planned): `{"content": "base64", "mode": "0644", "create_parents": true}`.
Delete returns `204`. Paths are interpreted against the session VFS (see
`crates/sandbox-vfs`).

### Snapshot (planned)

```
POST /v1/sessions/{id}/snapshot
```

Planned response: `201` with `id`, `parent`, `session`, `vfs_root` (BLAKE3
hex, matching `ContentHash` in `sandbox-types`), `tier`, `created_at`, `labels`.

### Fork snapshot (planned)

```
POST /v1/snapshots/{id}/fork
```

```json
{ "count": 3, "config_overrides": { "memory_limit_mb": 1024 } }
```

Planned response: `201` with a `sessions` array of `{id, snapshot}`.

## Rate limiting (planned)

Planned: per-key limits with `429` and `Retry-After`. Nothing enforces limits
yet. The `middleware.rs` module only adds a request ID.

## CLI

The `sandbox` binary (crate `cli`, source `cli/src/`) runs commands in an
in-process isolate session. It does not call the server.

```bash
sandbox run "echo hi | cat > /a && cat /a"  # args are joined and run as one script
sandbox shell                               # one script per stdin line; state persists
sandbox snapshot <session-id>     # returns an error; not implemented yet
sandbox fork <snapshot-id> 3      # returns an error; not implemented yet
sandbox replay <trace-file>       # returns an error; not implemented yet
sandbox serve --addr 0.0.0.0:8080 # returns an error; not implemented yet
sandbox bench [name]              # returns an error; not implemented yet
```

`run` exits with the script's exit code. Command output goes to stdout and
stderr. Only builtins run. Other commands exit 127 with a message. See
`crates/sandbox-shell/src/lib.rs` for the supported shell syntax.

`sandbox-server` has no binary target (`crates/sandbox-server/src` has only
`lib.rs`, `api.rs`, `auth.rs`, `middleware.rs`), so there is currently no way to
run the HTTP routes.

## SDKs

Status of each client:

- **Rust** (`sdks/rust`, crate `sandbox-sdk`): the HTTP client methods
  (`create_session`, `exec`, `snapshot`, `destroy_session`) return `Unsupported`.
  In embedded mode, `create_session` and `exec` also return `Unsupported`. The
  working in-process path is `sandbox-session`'s `IsolateBackend`, which the
  CLI uses directly.
- **TypeScript** (`sdks/typescript`, npm `sandbox-sdk`): has a fetch-based
  client, zod-validated types, and an embedded wrapper that spawns
  `sandbox-server`. Its paths do not match the server (see the known mismatch
  above).
- **Python** (`sdks/python`, package `sandbox_sdk`): httpx-based client with
  the same path mismatch, plus an embedded wrapper.

None of the SDKs has been run end to end against the server in this repo.

## OpenAPI

Planned, not yet generated. PLAN.md section 4.15 requires an OpenAPI spec
generated from types, with CI failing on drift. No spec file is committed.
