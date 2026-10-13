# sandbox-sdk (Rust)

Rust client SDK for Frontal Sandbox. The crate is `sandbox-sdk` (`sdks/rust/`).

**Status: scaffolding.** The public types exist, but the remote client is not implemented and the embedded mode does not execute code. See the per-method status below. Nothing here has been run against a real backend.

## Overview

Two modes are planned:

1. **Remote client**: HTTP to `sandbox-server`. Not implemented yet.
2. **Embedded mode**: in-process backend with no server, for tests and local use. Not implemented yet. The `embedded` cargo feature is declared, but the backend it is meant to enable is not wired up.

## API

Exports from `src/lib.rs`: `Client`, `EmbeddedSandbox`, and the modules `client` and `embedded`.

### `Client`

| Method | Status | Behavior |
|--------|--------|----------|
| `Client::embedded()` | Works (placeholder) | Returns a client whose base URL is `embedded://`. |
| `Client::from_env()` | Works | Reads `SANDBOX_BASE_URL` (default `http://localhost:8080`). It reads `SANDBOX_API_KEY` but does not use it. |
| `create_session(config: SessionConfig) -> Result<SessionId, SandboxError>` | Stub | Embedded client returns a new `SessionId`. Remote client returns `UNSUPPORTED` ("HTTP client not implemented"). |
| `exec(session_id: SessionId, command: &str) -> Result<ExecResult, SandboxError>` | Stub | Embedded client returns `ExecResult { exit_code: 0, stdout: "", stderr: "" }` without running anything. Remote client returns `UNSUPPORTED`. |
| `snapshot(session_id: SessionId) -> Result<SnapshotId, SandboxError>` | Stub | Always returns `UNSUPPORTED`. |
| `destroy_session(session_id: SessionId) -> Result<(), SandboxError>` | Stub | Always returns `UNSUPPORTED`. |

### `EmbeddedSandbox`

| Method | Status | Behavior |
|--------|--------|----------|
| `EmbeddedSandbox::new(config: SessionConfig) -> Result<Self, SandboxError>` | Stub | Creates an embedded client and a session ID. No backend is started. |
| `exec(&self, command: &str) -> Result<ExecResult, SandboxError>` | Stub | Delegates to `Client::exec`, so it returns the fake success result above. |
| `snapshot(&self) -> Result<SnapshotId, SandboxError>` | Stub | Returns `UNSUPPORTED`. |
| `destroy(&self) -> Result<(), SandboxError>` | Stub | Returns `UNSUPPORTED`. |

`ExecResult` (in `client.rs`) has public fields `exit_code: i32`, `stdout: String`, and `stderr: String`.

## Example

This compiles against the current signatures (not verified by a build in this refresh). Its results are placeholders today.

```rust
use sandbox_sdk::{Client, EmbeddedSandbox};
use sandbox_types::SessionConfig;

# async fn demo() -> Result<(), sandbox_types::SandboxError> {
// Embedded placeholder: returns exit code 0 and empty output; does not execute.
let client = Client::embedded()?;
let session = client.create_session(SessionConfig::default()).await?;
let out = client.exec(session, "echo hi").await?;
println!("{} {}", out.exit_code, out.stdout);

let sb = EmbeddedSandbox::new(SessionConfig::default()).await?;
let out = sb.exec("echo hi").await?;
sb.destroy().await?; // returns UNSUPPORTED today
# Ok(())
# }
```

## Not present

These do not exist in the crate. Do not use them:

- `Client::create`
- `SessionConfig::tier(...)`. Use `SessionConfig::with_tier(...)` from `sandbox-types` instead.
- `sb.fs()`, `sb.exec_stream()`, `sb.watch()`, `snapshot.fork()`
- `EmbeddedSandbox::new()` with no arguments

## Related

- `crates/sandbox-types/`: `SessionConfig`, `SessionId`, `SnapshotId`, and `SandboxError`.
- `crates/sandbox-server/`: the server these methods will call. Its handlers are placeholders today.
- `sdks/typescript/` and `sdks/python/`: the other SDKs. Their HTTP paths do not match the server (see their READMEs).
- `PLAN.md`: target API and milestones.
