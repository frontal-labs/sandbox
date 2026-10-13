# sandbox-sdk (Python)

Python client SDK for Frontal Sandbox. The distribution name is `sandbox-sdk` and the import name is `sandbox_sdk`. Source is in `sdks/python/sandbox_sdk/`.

**Status: early.** The types, error helpers, and an async HTTP client are written. The HTTP paths do not match `sandbox-server`, `create_session` is likely to fail on serialization (see Known issues), and the server's handlers return placeholder strings. "Embedded" mode does not start a local backend. Nothing here has been run against a live server.

## Installation

The package is not published to PyPI. Install from the repo:

```bash
pip install -e sdks/python
```

Note: `pyproject.toml` sets `[tool.hatch.build.targets.wheel] packages = ["src/sandbox_sdk"]`, but the package lives at `sdks/python/sandbox_sdk/`. That path does not exist, so the editable install is expected to fail until the build config is fixed. This was not verified by a build.

Requires Python 3.11+. Runtime dependencies: `httpx`, `pydantic`, `ulid-py`, `python-dotenv`.

## Quick Start

This shows the intended usage. It will not work end to end until the known issues below are fixed.

```python
import asyncio
import os

from sandbox_sdk import Client, ExecRequest, SandboxError, SessionConfig, is_sandbox_error

async def main() -> None:
    # Configure via environment variables (or pass ClientConfig explicitly)
    os.environ["SANDBOX_BASE_URL"] = "http://localhost:8080"
    os.environ["SANDBOX_API_KEY"] = "your-api-key"

    async with Client.from_env() as client:
        config = SessionConfig.new(tenant_id="my-tenant")
        session_id = await client.create_session(config)
        if is_sandbox_error(session_id):
            raise session_id

        result = await client.exec(session_id, ExecRequest(argv=["echo", "hello"]))
        if is_sandbox_error(result):
            print(f"Error [{result.code}]: {result.message}")
        else:
            print(result.stdout.decode())

        await client.destroy_session(session_id)

asyncio.run(main())
```

Errors are returned as values, not raised. Check each result with `is_sandbox_error()`.

## Embedded Mode (not implemented as described)

`EmbeddedSandbox` does not start a local backend. It is a thin wrapper over `Client`:

- `EmbeddedSandbox.new()` checks `http://localhost:8080/health` when `SANDBOX_BASE_URL` is unset. It returns `unavailable` if the check fails. The server does not route `/health` yet.
- `server_path` on `EmbeddedSandboxConfig` is accepted but not used.
- `exec(command)` runs `sh -c <command>` through the remote `Client`, so it needs a running server.

Local in-process execution is planned. See `PLAN.md`.

```python
import asyncio

from sandbox_sdk import EmbeddedSandbox, EmbeddedSandboxConfig, is_sandbox_error

async def main() -> None:
    sandbox = await EmbeddedSandbox.new(EmbeddedSandboxConfig(tenant_id="local-dev"))
    if is_sandbox_error(sandbox):
        raise sandbox

    result = await sandbox.exec("echo hi")
    print(result)
    await sandbox.destroy()

asyncio.run(main())
```

## Streaming Execution

`exec_stream` POSTs with `?stream=true` and parses newline-delimited JSON. Each line must deserialize into `ExecResult`. The server does not define a streaming chunk format yet.

```python
from sandbox_sdk import ExecRequest, is_sandbox_error

async for chunk in client.exec_stream(
    session_id,
    ExecRequest(argv=["python3", "-c", "print('hi')"], stream=True),
):
    if is_sandbox_error(chunk):
        print(f"Error: {chunk}")
    else:
        print(chunk.stdout.decode(), end="")
```

## API Reference

All methods are `async`. Each returns `Result[T]`, which is `T | SandboxError`.

### Client

- `Client.from_env()`: reads `SANDBOX_BASE_URL` (default `http://localhost:8080`) and `SANDBOX_API_KEY`
- `Client(ClientConfig(base_url=..., api_key=...))`: explicit construction
- `await client.close()`, or use `async with Client(...) as client:`
- `create_session(config: SessionConfig) -> Result[str]`
- `get_session(session_id) -> Result[SessionState]`
- `destroy_session(session_id) -> Result[None]`
- `exec(session_id, request: ExecRequest) -> Result[ExecResult]`
- `exec_stream(session_id, request: ExecRequest)`: async generator of `Result[ExecResult]`
- `snapshot(session_id) -> Result[Snapshot]`
- `list_snapshots(session_id) -> Result[list[Snapshot]]`
- `get_snapshot(session_id, snapshot_id) -> Result[Snapshot]`

### Types

Defined in `sandbox_sdk/types.py`:

- `SessionConfig`: frozen dataclass. Build with `SessionConfig.new(tenant_id=...)` and `with_tier`, `with_limits`, `with_network`, `with_env`, `with_cwd`, `with_snapshot`, `with_secrets`, `with_label`.
- `ExecRequest(argv, env=..., cwd=None, stdin=None, timeout_ms=None, stream=False)`
- `ExecResult(exit_code, stdout: bytes, stderr: bytes, cpu_ms, wall_ms)`
- `SessionState`, `Snapshot`, `ResourceLimits`, `NetworkPolicy`, `NetworkPolicyRule`, `SecretEntry`, `SecretsConfig`
- `Tier`, `TierPreference`, `SessionStatus` (Literal aliases)

### Errors

`SandboxError` is an exception class with a `code` and `message`. Helper constructors (`internal`, `invalid_argument`, `unavailable`, and others) return it. `is_sandbox_error(value)` tests whether a value is one. Import them from `sandbox_sdk`.

## Known issues

- **Snapshot list and get are not routed.** `list_snapshots` and `get_snapshot` call `/v1/sessions/:id/snapshots` routes that `crates/sandbox-server/src/api.rs` does not define, so they will 404. `snapshot()` uses the routed `POST /v1/sessions/:id/snapshot`.
- **`create_session` serialization.** The client sends `config.__dict__`. `SessionConfig` contains nested dataclasses (`ResourceLimits`, `NetworkPolicy`), which `httpx`'s JSON encoder cannot serialize. The resulting `TypeError` is caught and returned as an `INTERNAL` error. This was found by reading the code, not by running it.
- **`ExecResult` from JSON.** The client builds `ExecResult(**json)`, but JSON has no `bytes` type, so `stdout` and `stderr` will not be `bytes` unless the server encodes them in a way the client decodes. The wire format is not defined yet.
- **Server placeholders.** Every server handler returns a fixed string.
- **Not routed on the server:** session list, `list_snapshots`, `get_snapshot`, and `/health`.
- **Packaging.** The wheel `packages` path does not match the directory layout (see Installation).
- **Tests.** `tests/test_types.py` covers types, errors, and IDs only. There are no client tests.

## Development

```bash
# Install with dev dependencies (from the repo root)
pip install -e "sdks/python[dev]"

# Run tests
pytest sdks/python/tests

# Type check
mypy sdks/python/sandbox_sdk

# Lint and format (ruff config is in pyproject.toml; indent width 2)
ruff check sdks/python/sandbox_sdk
ruff format sdks/python/sandbox_sdk
```

None of these commands, nor the install, were run as part of this doc refresh.

## License

Apache-2.0
