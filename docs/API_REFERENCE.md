# API Reference

> **Status: implemented.** Every route in the table below is routed and
> exercised by the integration test suite. All routes require a valid API key.
> The full target API is specified in the project specification section 4.15;
> this page describes what the server actually serves today.

## Base URL

`sandbox serve` listens on `0.0.0.0:8080` by default (override with
`--addr`). Routes are mounted under `/v1`, so the base URL is:

```
http://localhost:8080/v1
```

The SDKs default to `SANDBOX_BASE_URL=http://localhost:8080`.

**Wire format:** field names are the server's snake_case. Timestamps are
RFC 3339 strings. Byte fields (`stdin`, `stdout`, `stderr`) are arrays of
0-255 integers, not base64. All IDs are 26-character Crockford ULIDs
(`^[0-9A-HJKMNP-TV-Z]{26}$`) — session, snapshot, and template alike; there is
no `sess_` / `snap_` prefix. Content hashes are 64-character BLAKE3 hex.

## Routed endpoints

| Method | Path | Handler | Behavior |
|--------|------|---------|----------|
| POST | `/v1/sessions` | `create_session` | Create a session |
| GET | `/v1/sessions/{id}` | `get_session` | Status, tier, checkpoints |
| DELETE | `/v1/sessions/{id}` | `destroy_session` | Destroy a session |
| POST | `/v1/sessions/{id}/exec` | `exec_command` | Run a command (buffered or streamed) |
| GET | `/v1/sessions/{id}/ws` | `exec_websocket` | Exec over WebSocket |
| GET | `/v1/sessions/{id}/pty` | `pty_websocket` | PTY over WebSocket |
| GET | `/v1/sessions/{id}/fs/{path}` | `read_file` | Read a file |
| PUT | `/v1/sessions/{id}/fs/{path}` | `write_file` | Write a file |
| DELETE | `/v1/sessions/{id}/fs/{path}` | `delete_file` | Delete a file or empty directory |
| GET | `/v1/sessions/{id}/trace` | `get_trace` | The session's command trace (ADR 0007) |
| GET | `/v1/sessions/{id}/info` | `session_info` | The session's `/etc/agent/env.json` |
| POST | `/v1/sessions/{id}/sleep` | `sleep_session` | Checkpoint, then release the live session (ADRs 0006, 0010) |
| POST | `/v1/sessions/{id}/wake` | `wake_session` | Restore a sleeping session under the same ID |
| POST | `/v1/sessions/{id}/checkpoint` | `checkpoint_session` | Pin the current state as a checkpoint |
| POST | `/v1/sessions/{id}/undo` | `undo_session` | Restore `steps` checkpoints back |
| POST | `/v1/sessions/{id}/fork` | `fork_command` | Fork the session into N children that each run a command |
| POST | `/v1/sessions/{id}/snapshot` | `take_snapshot` | Snapshot the committed filesystem |
| GET | `/v1/sessions/{id}/snapshots` | `list_snapshots` | Snapshots taken from a session |
| GET | `/v1/sessions/{id}/snapshots/{snapshot_id}` | `get_snapshot` | One snapshot taken from a session |
| POST | `/v1/snapshots/{id}/fork` | `fork_snapshot` | Start 1 to 64 sessions from a snapshot |
| POST | `/v1/templates` | `create_template` | Create a named template over a snapshot (ADR 0049) |
| GET | `/v1/templates` | `list_templates` | This key's templates |
| GET | `/v1/templates/{id}` | `get_template` | One template |
| POST | `/v1/gc` | `run_gc` | Start a blob-store collection in the background (ADR 0013) |
| GET | `/v1/usage` | `get_usage` | The caller's counters since the server started (ADR 0015) |
| GET | `/v1/usage/export` | `get_usage_export` | Export the caller's usage rows |
| GET | `/v1/billing/events` | `get_billing_events` | Billing events for the caller |
| GET | `/v1/openapi.json` | `openapi_json` | Generated OpenAPI document (no auth) |
| GET | `/metrics` | `get_metrics` | Prometheus text format (requires a key) |

Source: `crates/sandbox-server/src/api.rs` (`create_router`).

## Contract files

- REST: `openapi/v1/openapi.json` and `openapi/v1/openapi.yaml`, generated from
  `crates/sandbox-server/src/api.rs`. Every route and its responses are listed
  there.
- Internal and gRPC: `proto/sandbox/v1/`, compiled into `crates/sandbox-proto`.
  The worker service is live; the `api` services are schema only.

## Authentication

Every `/v1` route requires `Authorization: Bearer <key>`. Missing or invalid
keys get `401 Unauthorized`. `GET /v1/openapi.json` is the only unauthenticated
route.

Keys have the form `sbx_<key-id>_<secret>`. `AuthManager::create_key` returns
the plaintext once; `sandbox serve` prints it to stderr on first start. The
store keeps only a BLAKE3 digest of the secret, and `revoke` removes a key by
id. Per-key scopes and quotas are still planned (project specification
section 4.15).

## Error format

Invalid request bodies — a malformed JSON body, or a well-formed JSON object
missing a required field — are rejected by the framework before the handler
runs, with `422` and a deserialization message in the body:

```json
{ "message": "Failed to deserialize the JSON body into the target type: missing field `tier_preference` ..." }
```

Other errors render as an `ErrorBody` with the HTTP status the code implies
(`ApiError::into_response` in `api.rs`):

```json
{ "code": "NOT_FOUND", "message": "Human-readable description" }
```

| `ErrorCode` | HTTP status |
|-------------|-------------|
| `NotFound` | `404` |
| `InvalidArgument`, `ShellError` | `400` |
| `AlreadyExists`, `InvalidState` | `409` |
| `PermissionDenied`, `PolicyDenied` | `403` |
| `ResourceExhausted` | `429` (with `Retry-After`) |
| `Unsupported` | `501` |
| `Timeout` | `504` |
| `Unavailable`, `NeedsVm` | `503` |
| anything else | `500` |

The vocabulary lives in `crates/sandbox-types/src/error.rs`. A `429` also
carries a `Retry-After` header in seconds.

## Create a session

```
POST /v1/sessions
```

The body is a full `SessionConfig`. Fields without `#[serde(default)]` are
required on the wire — omitting one is a `422` (Axonum's untagged request
deserialization failure), not a default. `checkpoint` (default `per_exec`)
and `checkpoint_retain` (default 100) are the only optional ones. The
minimal working body:

```json
{
  "tenant_id": "01K0D8X0000000000000000001",
  "tier_preference": "isolate",
  "limits": {
    "memory_bytes": 536870912,
    "cpu_ms_per_sec": 1000,
    "wall_timeout_secs": 300,
    "max_file_size": 104857600,
    "max_total_size": 1073741824,
    "max_inodes": 100000,
    "max_path_depth": 32,
    "max_processes": 128,
    "max_open_files": 256
  },
  "network": { "default_deny": true, "rules": [] },
  "env": {},
  "cwd": "/work",
  "labels": {},
  "checkpoint_retain": 50
}
```

Field notes:

- `tenant_id` — a 26-character Crockford ULID.
- `tier_preference` — `auto` (default), `isolate`, or `vm`. A `vm` session
  needs a configured guest pool, else `503` (`NEEDS_VM`).
- `limits` — the `ResourceLimits` struct; every field is an `Option`, so the
  field must be present but its value may be `null`. Enforced limits:
  `max_file_size`, `max_total_size`, `max_inodes`, `max_path_depth`
  (ADR 0014).
- `network` — `default_deny` plus a `rules` array. See
  [LIMITS.md](LIMITS.md) for what each limit enforces.
- `checkpoint` — `per_exec` (default) or `manual`; omitted means `per_exec`.
- `checkpoint_retain` — `1` to `MAX_CHECKPOINT_RETAIN`; the default (100)
  applies when the field is omitted.
- `initial_snapshot` / `initial_template` — start the session from an existing
  snapshot or template. Set at most one; both is `400` (`InvalidArgument`).

Response: `201` with `{"session_id": "<ULID>"}`.

## Get and destroy a session

```
GET    /v1/sessions/{id}
DELETE /v1/sessions/{id}
```

`GET` returns a `SessionView`: the flattened `SessionState` (`id`, `tenant_id`,
`tier`, `allow_vm`, `status`, `config_hash`, `vfs_root`, `created_at`,
`updated_at`, `labels`) plus `checkpoints` (`policy`, `count`, `retain`,
`bytes`). `DELETE` returns `204`.

## Execute a command

```
POST /v1/sessions/{id}/exec
```

```json
{
  "argv": ["sh", "-c", "echo hi"],
  "env": {},
  "cwd": "/work",
  "stdin": [104, 101, 108, 108, 111, 10],
  "timeout_ms": 60000,
  "stream": false,
  "output_limit": null
}
```

- `argv` and `env` are required; `cwd`, `stdin`, `timeout_ms`, and
  `output_limit` are optional.
- `stdin` is an integer array, not base64.
- `timeout_ms` is enforced on the isolate tier. A command that runs past it
  returns `TIMEOUT` (HTTP 504), and the writes it made before stopping stay in
  the filesystem.

Buffered response: `200` with an `ExecResult`:

```json
{
  "exit_code": 0,
  "stdout": [104, 105, 10],
  "stderr": [],
  "cpu_ms": 5,
  "wall_ms": 12
}
```

`stdout` and `stderr` are integer arrays, capped to `output_limit` (default
8 KiB head + 8 KiB tail); the full stream stays in the trace.

Streaming (`?stream=true`, or `"stream": true` in the body) returns
`application/x-ndjson`, one JSON object per line:

```json
{"type": "output", "stream": "stdout", "data": [104, 105, 10]}
{"type": "output", "stream": "stderr", "data": [110, 111, 112, 101, 10]}
{"type": "exit", "exit_code": 0, "cpu_ms": 5, "wall_ms": 12}
```

Exactly one `exit` or `error` line ends the response. An `error` line has the
`ErrorBody` fields and is how a command that times out or fails after its
output started is reported, because the status is already 200. A request that
fails before streaming starts — an unknown session, a missing field — is an
ordinary HTTP error. See [ADR 0020](adr/0020-streaming-exec.md).

A command the isolate tier cannot run (it needs a real VM) returns `503`
(`NEEDS_VM`) when no guest pool is configured. When a pool is configured and
the session is promotable, the server moves the session to a VM and runs the
command there — the session keeps its ID, and its `tier` flips to `vm`.

## Filesystem

```
GET    /v1/sessions/{id}/fs/{path}
PUT    /v1/sessions/{id}/fs/{path}
DELETE /v1/sessions/{id}/fs/{path}
```

Paths are relative to the session VFS root, with no leading slash; `..`
segments are refused (`InvalidArgument`).

- `GET` returns the raw bytes as `application/octet-stream`. A file larger
  than the read limit returns `413`. A directory path returns `InvalidArgument`.
- `PUT` takes the raw bytes in the body (not a JSON envelope) and returns
  `204`.
- `DELETE` returns `204`. A non-empty directory or a missing path is an error.

There is no directory-listing route.

## Command trace

```
GET /v1/sessions/{id}/trace
```

Response: `200`, `application/x-ndjson`, in the ADR 0007 format. Each command
the session ran through `exec` is one run, with its argv written as a
single-quoted script. The `X-Trace-Complete` header is `false` when a command
could not be recorded (it had standard input, or an environment name is not a
shell name) or when the trace is full (4096 runs, or 8 MiB). Each run's
`files` field lists the files the command changed, with the operation and the
content hashes before and after; a rename appears as a delete and a create,
and a chmod is not reported. Writes and deletes through the filesystem API are
runs of their own, with the written bytes in an `fs` field. Save the body and
replay it with `sandbox replay`, which makes the same writes again. See
[ADR 0009](adr/0009-server-exec-traces.md),
[ADR 0011](adr/0011-file-changes-in-traces.md), and
[ADR 0012](adr/0012-filesystem-writes-in-traces.md).

## Session info

```
GET /v1/sessions/{id}/info
```

Returns the session's `/etc/agent/env.json` (schema 1) — the tier, the
effective limits, and the network allow-list the agent was started with. This
is what the in-session `sandbox`/agent tooling reads to know its environment.

## Checkpoint and undo

```
POST /v1/sessions/{id}/checkpoint
POST /v1/sessions/{id}/undo
```

`checkpoint` pins the current state as a checkpoint now. A `per_exec` session
also pins after each command. Response: `200` with a `CheckpointResponse`:
`root` (the BLAKE3 hash of the pinned tree) and `checkpoints`
(`policy`, `count`, `retain`, `bytes`).

`undo` restores the filesystem from `steps` checkpoints back (body
`{"steps": N}`, default 1). The current state is pinned first, so undo can
return to it. A `manual` session can only undo to a checkpoint it took
explicitly. Response: `200` with the same `CheckpointResponse`.

## Fork a session

```
POST /v1/sessions/{id}/fork
```

```json
{ "count": 3, "argv": ["sh", "-c", "echo hi > /work/child.txt"] }
```

Forks the session's current state into `count` children (1 to 64) and runs
`argv` in each. Each child is an independent session, and its command is
recorded in the child's own trace. Response: `200` with
`{"children": [{"session_id", "exit_code", "error", "changed_files"}, ...]}`.
A clean run has `exit_code` set and `error` `null`; when the command could
not run, `exit_code` is `null` and `error` carries the reason.

## Snapshots

```
POST /v1/sessions/{id}/snapshot
GET  /v1/sessions/{id}/snapshots
GET  /v1/sessions/{id}/snapshots/{snapshot_id}
POST /v1/snapshots/{id}/fork
```

`snapshot` returns `201` with `{"snapshot_id": "<ULID>"}`. The record is read
back with `GET .../snapshots/{snapshot_id}`, which returns a `Snapshot`:
`id`, `parent`, `vfs_root` (BLAKE3 hex), `env`, `cwd`, `tier`,
`runtime_blob`, `created_at`, `labels`.

`fork` starts 1 to 64 new sessions from a snapshot. Body `{"count": N}`.
Response: `200` with `{"session_ids": ["<ULID>", ...]}`. The caller must own
the snapshot.

## Templates

A template is a stable name over an existing snapshot: it captures the
snapshot's committed filesystem root so a session can start from a pre-built
filesystem (ADR 0049). It is not a snapshot renamed; it layers ownership and a
name on top of a snapshot record.

```
POST /v1/templates          {"name": "build", "snapshot_id": "<ULID>"}
GET  /v1/templates
GET  /v1/templates/{id}
```

`POST` returns `201` with the `Template`. A session starts from one with
`"initial_template": "<ULID>"` in its `SessionConfig`.

## Sleep and wake

```
POST /v1/sessions/{id}/sleep
POST /v1/sessions/{id}/wake
```

Sleep checkpoints the filesystem, records it as a snapshot under the caller's
key, and releases the live session, so a sleeping session holds no filesystem
in memory. Response: `200` with `{"snapshot_id": "<ULID>"}`. The session's
status is `sleeping`.

Wake restores the session from the checkpoint that sleep recorded. Response:
`200` with the `SessionState`, status `running`. The session keeps its ID.
Files, and the working directory, come back. The environment does not, because
its values often hold secrets (ADR 0010).

A request to a sleeping session wakes it first, so `exec`, file operations,
and `snapshot` succeed. `get` and `destroy` never wake it. Sleeping a sleeping
session, or waking a running one, returns `409` with code `INVALID_STATE`.
Idle sessions sleep by themselves when `sandbox serve --idle-sleep-secs` is
set (default 300). See
[ADR 0006](adr/0006-sleep-and-wake.md) and
[ADR 0015](adr/0015-idle-sleep-and-metering.md).

## Garbage collection and usage

```
POST /v1/gc
GET  /v1/usage
GET  /v1/usage/export
GET  /v1/billing/events
```

`gc` starts a blob-store collection in the background and returns `202`. At
most one runs per minute; a second request within the window is `429`.
`sandbox serve --gc-interval-secs` also runs one on a schedule (ADR 0013).

`usage` returns the caller's `Counters` since the server started:
`sessions_created`, `sessions_slept`, `sessions_woken`, `execs`,
`exec_wall_ms`, `bytes_written` (ADR 0015). `usage/export` returns the same
data as exportable rows; `billing/events` returns the caller's billing events.

## Rate limiting

Session admission is bounded by per-key, per-tenant, and server-wide limits.
When a request would exceed a limit, it is refused with `429`
(`RESOURCE_EXHAUSTED`) and a `Retry-After` header (in seconds), so clients can
back off instead of retrying into the limit. The per-key and server-wide limits
are enforced on `POST /v1/sessions` and on snapshot forks; the per-tenant limit
is enforced on top of them. The `middleware.rs` module only adds a request ID.

## WebSocket exec and PTY

```
GET /v1/sessions/{id}/ws
GET /v1/sessions/{id}/pty
```

`ws` runs an exec over WebSocket. The first client message is the JSON exec
request; the server sends each event as a text message, then a close frame
after `exit` or `error`. Stdin beyond the request's `stdin` field and signals
are not part of this protocol yet (ADR 0030). A plain HTTP request (no
upgrade) returns `400`.

`pty` is the same handshake for a pseudo-terminal. Both routes are registered
and require the WebSocket upgrade.

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
