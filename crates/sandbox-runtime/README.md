# sandbox-runtime

V8 host built on `deno_core` for JavaScript and TypeScript execution in the sandbox.

**Status: stub.** The crate creates a `deno_core` runtime and discards it. `Runtime::execute_script` returns an `unsupported` error. Most custom ops are placeholders. `op_env_set` mutates the process environment, which is shared across tenants (see below).

## Public API

Re-exported from `src/lib.rs`:

- `Runtime` (`src/runtime.rs`)
  - `new() -> Result<Self, SandboxError>`: builds and drops a `deno_core::JsRuntime`.
  - `async execute_script(&mut self, code: &str) -> Result<String, SandboxError>`: returns `unsupported`.
  - `Default` implementation (panics if `new` fails).
- `ops` module (`src/ops.rs`), `op2` functions:

| Op | State |
|----|-------|
| `op_now` | Real wall-clock millis (not virtual). |
| `op_env_get` | Reads the host process environment. |
| `op_env_set` | Writes the host process environment. |
| `op_fs_read`, `op_fs_write`, `op_fs_stat`, `op_fs_readdir` | Placeholders. Read returns empty, write does nothing. Not wired to the VFS. |
| `op_spawn` | Placeholder. Returns 0. |
| `op_fetch` | Placeholder. Returns an empty string. Not wired to the policy client. |

The ops are not registered with a runtime yet.

## Planned design

- A `JsRuntime` facade over `deno_core` and `v8`, with exact version pins in the workspace `Cargo.toml`. The crate currently depends on `deno_core` `0.412` through the workspace.
- Extensions: `deno_webidl`, `deno_url`, `deno_web`, `deno_console`, `deno_crypto`, `deno_fetch`. `deno_fs`, `deno_net`, `deno_io`, and `deno_node` are excluded initially.
- Capability-checked ops that route to the VFS, the policy-checked HTTP client, and the shell.
- Startup snapshot, Node compatibility shim (M2+), TypeScript transpile on load, and limits (heap, CPU watchdog, wall timeout, op queue depth).

## Invariants and security assumptions

- Target: no direct host filesystem or network access from JS. Today this holds only because the fs and fetch ops are placeholders.
- `op_env_set` and `op_env_get` touch the real process environment. This must be replaced by a per-session environment before any multi-tenant use.
- Isolates must never be reused across tenants. This is planned for the pool, not implemented.

## Example

Not provided. `execute_script` currently always returns an error, so an example would not show working behaviour.

## Spec

See [PLAN.md](../../PLAN.md) section 4.5. Milestone M1 covers isolate, VFS, and shell core.
