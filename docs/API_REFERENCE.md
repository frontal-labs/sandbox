# API Reference

> **Status: scaffold.** The HTTP routes below are wired in
> `crates/sandbox-server/src/api.rs`, but every handler currently returns a
> fixed placeholder string (for example `"create_session"`). No request body is
> parsed, no response is JSON, no session is created, and no auth is enforced.
> The JSON shapes in this document are the intended contract, not current
> behavior. The full target API is specified in
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

**Known gap:** both SDKs now call `/v1/...`, matching the server. The snapshot
take call uses `POST /v1/sessions/{id}/snapshot`. The SDKs also call
`GET /v1/sessions/{id}/snapshots` and `GET /v1/sessions/{id}/snapshots/{snapshot_id}`,
which the server does not route yet, so those calls will return 404.

## Routed endpoints

| Method | Path | Handler (placeholder) | Notes |
|--------|------|-----------------------|-------|
| POST | `/v1/sessions` | `create_session` | Create a session |
| GET | `/v1/sessions/{id}` | `get_session` | Status, tier |
| DELETE | `/v1/sessions/{id}` | `destroy_session` | Destroy a session |
| POST | `/v1/sessions/{id}/exec` | `exec_command` | Run a command |
| GET | `/v1/sessions/{id}/fs/{path}` | `read_file` | Read a file |
| PUT | `/v1/sessions/{id}/fs/{path}` | `write_file` | Write a file |
| DELETE | `/v1/sessions/{id}/fs/{path}` | `delete_file` | Delete a file |
| POST | `/v1/sessions/{id}/snapshot` | `take_snapshot` | Snapshot a session |
| POST | `/v1/snapshots/{id}/fork` | `fork_snapshot` | Fork a snapshot |

Source: `crates/sandbox-server/src/api.rs` (`create_router`).

## Planned, not routed

These appear in PLAN.md section 4.15 but have no route in `api.rs`:

- `GET /v1/sessions/{id}/ws`: interactive WebSocket
- `GET /v1/sessions/{id}/fs?list=...`: directory listing
- `GET /v1/sessions/{id}/trace`: trace event stream
- `GET /v1/usage`: metering
- `GET /v1/openapi.json`: OpenAPI spec (planned via `utoipa`; `utoipa` is a
  workspace dependency but no spec is generated yet)

## Authentication

Planned: API keys, hashed at rest and prefix-identified, with per-key scopes
and quotas (PLAN.md section 4.15). `crates/sandbox-server/src/auth.rs` contains
a `// TODO: Implement API key validation`, and `create_router` does not apply
any auth layer. Requests are currently unauthenticated.

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

The `sandbox` binary (crate `cli`, source `cli/src/`) defines these
subcommands. In the current code none of them calls the server or runs a
command; each prints a message, and `run` and `shell` are marked `TODO` in
`cli/src/commands.rs`.

```bash
sandbox run "npm test"            # prints a message; execution TODO
sandbox shell                     # prints a message; shell TODO
sandbox snapshot <session-id>     # prints a message
sandbox fork <snapshot-id> 3      # prints a message
sandbox replay <trace-file>       # prints a message
sandbox serve --addr 0.0.0.0:8080 # prints "Starting server"; does not start one
sandbox bench [name]              # prints a message
```

`sandbox-server` has no binary target (`crates/sandbox-server/src` has only
`lib.rs`, `api.rs`, `auth.rs`, `middleware.rs`), so there is currently no way to
run the HTTP routes.

## SDKs

Status of each client:

- **Rust** (`sdks/rust`, crate `sandbox-sdk`): `client.rs` methods
  (`create_session`, `exec`, `snapshot`, `destroy_session`) are stubs that
  ignore their arguments. `embedded.rs` exposes `EmbeddedSandbox::new`, `exec`,
  `snapshot`, and `destroy`, but these are not backed by a working isolate
  backend (`sandbox-session` `IsolateBackend` is `unimplemented!()`).
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
