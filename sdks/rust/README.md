# Sandbox SDK (Rust)

Rust client for `sandbox-server`. Every call is one HTTP request to the `/v1` API.

**Status: early.** The client covers sessions, exec, files, snapshots, and forks. It has no retries, no streaming exec, and no WebSocket. It is not published.

## Configuration

`Client::from_env()` reads:

- `SANDBOX_BASE_URL`, default `http://localhost:8080`
- `SANDBOX_API_KEY`, sent as a bearer token. The server issues keys at startup (`sandbox serve` prints one).

`Client::new(base_url, api_key)` takes both explicitly.

## Methods

| Method | Route | Returns |
|--------|-------|---------|
| `create_session(SessionConfig)` | `POST /v1/sessions` | `SessionId` |
| `exec(session, "cmd args")` | `POST /v1/sessions/{id}/exec` | `ExecResult` (text streams) |
| `exec_request(session, ExecRequest)` | `POST /v1/sessions/{id}/exec` | `sandbox_types::ExecResult` (bytes) |
| `read_file` / `write_file` / `delete_file` | `/v1/sessions/{id}/fs/{path}` | bytes, or `()` |
| `snapshot(session)` | `POST /v1/sessions/{id}/snapshot` | `SnapshotId` |
| `list_snapshots(session)` | `GET /v1/sessions/{id}/snapshots` | `Vec<Snapshot>` |
| `get_snapshot(session, snapshot)` | `GET /v1/sessions/{id}/snapshots/{snapshot_id}` | `Snapshot` |
| `fork(snapshot, count)` | `POST /v1/snapshots/{id}/fork` | `Vec<SessionId>` |
| `destroy_session(session)` | `DELETE /v1/sessions/{id}` | `()` |

`exec` splits the command on whitespace. Use `exec_request` for arguments with spaces, an environment, or stdin.

## Errors

A non-2xx response becomes a `SandboxError` with the server's `code` and `message`. Authentication failures are `PermissionDenied`. An unknown session, or one owned by another key, is `NotFound`. A refused setting is `Unsupported`. A server that cannot be reached is `Unavailable`.

## Example

```rust
use sandbox_sdk::Client;
use sandbox_types::{SessionConfig, TenantId, TierPreference};

# async fn demo() -> Result<(), sandbox_types::SandboxError> {
let client = Client::from_env()?;
let session = client
    .create_session(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Isolate,
        ..SessionConfig::default()
    })
    .await?;

let out = client.exec(session, "echo hello").await?;
assert_eq!(out.stdout.trim(), "hello");

client.write_file(session, "/notes.txt", b"saved".to_vec()).await?;
let snapshot = client.snapshot(session).await?;
let forks = client.fork(snapshot, 2).await?;
assert_eq!(client.read_file(forks[0], "/notes.txt").await?, b"saved");

client.destroy_session(session).await?;
# Ok(())
# }
```

`EmbeddedSandbox` wraps one session on a client for scripts that only need one:

```rust
use sandbox_sdk::{Client, EmbeddedSandbox};
# async fn demo() -> Result<(), sandbox_types::SandboxError> {
let sandbox = EmbeddedSandbox::new(Client::from_env()?, sandbox_types::SessionConfig::default()).await?;
let out = sandbox.exec("pwd").await?;
sandbox.destroy().await?;
# Ok(())
# }
```

## Tests

- Unit tests in `src/` cover the path encoding and the base URL.
- `tests/against_server.rs` starts the real router on a local port and drives every method over HTTP. It covers the file round trip, deletion, destroy, snapshots, fork, a refused key, and a session that another key cannot reach.

```sh
cargo test -p sandbox-sdk
bazel test //sdks/rust:sandbox_sdk_unit_test //sdks/rust:sandbox_sdk_test
```

## Related

- `crates/sandbox-server/`: the server these methods call. Its routes and limits are in its README.
- `crates/sandbox-types/`: `SessionConfig`, `SessionId`, `SnapshotId`, `Snapshot`, and `SandboxError`.
- `sdks/typescript/` and `sdks/python/`: the other SDKs. They use the same snake_case wire format.
