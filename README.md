# Frontal Sandbox (experimental)

![Frontal Banner](./banner.jpg)

**Millisecond sandboxes for AI agents**

Frontal Sandbox provides instant, low-cost execution environments for AI agents.
Sessions begin in V8-fast isolates and promote to real Linux microVMs only when a
command requires them.

## Product direction

- Tier 1 isolate sessions with warm-pool startup under 5 ms.
- Tier 2 Firecracker microVM promotion for commands that need a real binary.
- A copy-on-write, content-addressed virtual filesystem shared by every backend.
- Snapshot, fork, restore, and replayable traces for every command and file change.
- A small API: `create`, `exec`, `fs`, `snapshot`, `fork`, and `destroy`.
- Capability-based access with network denied by default.

## Architecture

```text
Client (Rust SDK / TypeScript SDK / CLI)
  -> server -> scheduler -> worker
                         -> isolate backend (shell, V8, WASM, VFS)
                         -> VM backend (Firecracker + guest agent)
  snapshot store + trace sink
```

The VFS is the source of truth. The SDK does not need to know which execution
tier is active; tier is exposed as read-only session state.

## Repository layout

```text
crates/                     Rust workspace members (crate names are sandbox-*)
├── sandbox-types           Shared IDs, errors, events, and configuration
├── sandbox-vfs             Copy-on-write virtual filesystem
├── sandbox-cas             Content-addressed blob storage
├── sandbox-shell           POSIX-ish shell and builtins
├── sandbox-runtime         Deno/V8 isolate host
├── sandbox-wasm            Wasmtime and WASI tool host
├── sandbox-session         Session trait and backends
├── sandbox-snapshot        Snapshot, fork, and restore
├── sandbox-vmm             Firecracker driver
├── sandbox-guest           In-VM guest agent
├── sandbox-promote         Isolate-to-VM handoff
├── sandbox-policy          Capabilities, network policy, and limits
├── sandbox-trace           Event log, diff, and replay
├── sandbox-worker          Per-tenant worker process
├── sandbox-scheduler       Placement, pools, sleep, and metering
└── sandbox-server          HTTP and WebSocket API
cli/                        Command-line client (crate `cli`, binary `frontal-sandbox`)
sdks/
├── rust/                   Rust client SDK (crate `sandbox-sdk`)
├── typescript/             TypeScript SDK (npm `@frontal-labs/sandbox-sdk`, built with Bazel)
└── python/                 Python SDK (package `sandbox_sdk`)
benchmarks/                 Criterion benches and load tests
docs/                       Architecture, security model, API, and limits
```

Most crates are early scaffolds or stubs. The server runs session, exec, filesystem, snapshot, and fork routes against in-memory sessions; snapshot and fork also run against in-memory sessions. `frontal-sandbox run` and `frontal-sandbox shell` are not yet implemented. See [PLAN.md](PLAN.md) for the milestone status,
the complete design specification, and target behavior.

## Development

Rust (workspace root `Cargo.toml`, toolchain pinned in `rust-toolchain.toml`):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

TypeScript and repository tooling (root `package.json`; the `ts:*` scripts run
through Bazel):

```bash
bun install
bun run lint          # biome check
bun run ts:build      # bazel build //sdks/typescript:all
bun run ts:test       # bazel test //sdks/typescript:test
```

Contribution guidelines are in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache-2.0
