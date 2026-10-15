# API Reference

> **Status: early.** Session, exec, filesystem, snapshot, fork, sleep, and wake
> routes work against in-memory sessions, and every `/v1` route requires a valid
> API key.
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
| POST | `/v1/sessions/{id}/sleep` | `sleep_session` | Snapshot, then release the live session (ADR 0006) |
| POST | `/v1/sessions/{id}/wake` | `wake_session` | Restore a sleeping session under the same ID |
| GET | `/v1/sessions/{id}/trace` | `get_trace` | The commands the session ran, as an ADR 0007 trace (ADR 0009) |
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

Response: `201` with `{ "snapshot_id": "..." }`. Read the record with
`GET /v1/sessions/{id}/snapshots/{snapshot_id}`, which returns `id`, `parent`,
`vfs_root` (BLAKE3 hex, matching `ContentHash`), `env`, `cwd`, `tier`,
`runtime_blob`, `created_at`, and `labels`.

### Sleep and wake

```
POST /v1/sessions/{id}/sleep
POST /v1/sessions/{id}/wake
```

Sleep checkpoints the filesystem, records it as a snapshot under the caller's
key, and releases the live session, so a sleeping session holds no filesystem in
memory. Response: `200` with `{ "snapshot_id": "..." }`. The session's status is
`sleeping`.

Wake restores the session from the root that sleep recorded. Response: `200`
with the session state, status `running`. The session keeps its ID. Files come
back. The working directory and environment start fresh, because shell state is
not part of a snapshot.

While a session sleeps, `exec`, file operations, and `snapshot` return `409` with
code `INVALID_STATE`. `get` and `destroy` still work. Sleeping or waking in the
wrong state returns `409`. See [ADR 0006](adr/0006-sleep-and-wake.md).

### Command trace

```
GET /v1/sessions/{id}/trace
```

Response: `200`, `application/x-ndjson`, in the ADR 0007 format. Each command the
session ran through `exec` is one run, with its argv written as a single-quoted
script. `X-Trace-Complete` is `false` when a command could not be recorded (it had
standard input, or an environment name is not a shell name) or when the trace is
full (4096 runs). File writes are not recorded. Save the body and replay it with
`sandbox replay`. See [ADR 0009](adr/0009-server-exec-traces.md).

### Fork snapshot

```
POST /v1/snapshots/{id}/fork
```

```json
{ "count": 3 }
```

Response: `200` with `{ "session_ids": [...] }`. `count` is 1 to 64. The caller
must own the snapshot.

## Rate limiting (planned)

Planned: per-key limits with `429` and `Retry-After`. Nothing enforces limits
yet. The `middleware.rs` module only adds a request ID.

## CLI

The `sandbox` binary (crate `cli`) runs commands in an in-process isolate
session, records and replays traces, and times the local runtime. `snapshot`
and `fork` call a server named by `SANDBOX_BASE_URL`. See
[cli/README.md](../cli/README.md) for every command.

```bash
sandbox run --trace one.jsonl "echo hi | cat > /a && cat /a"  # exit code = the script's
sandbox shell --trace session.jsonl                           # one script per stdin line
sandbox replay one.jsonl                                      # exit 1 on any divergence
sandbox bench --name exec_echo                                # one JSON line per case
sandbox serve --addr 127.0.0.1:8080                           # prints one API key to stderr
SANDBOX_BASE_URL=... SANDBOX_API_KEY=... sandbox snapshot <session-id>
SANDBOX_BASE_URL=... SANDBOX_API_KEY=... sandbox fork <snapshot-id> --count 3
```
