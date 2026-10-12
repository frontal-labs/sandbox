# Frontal Sandbox

![Frontal Banner](./banner.jpg)

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

## Planned layout

```text
crates/
├── sandbox-types       Shared IDs, errors, events, and configuration
├── sandbox-vfs         Copy-on-write virtual filesystem
├── sandbox-cas         Content-addressed blob storage
├── sandbox-shell       POSIX-ish shell and builtins
├── sandbox-runtime     Deno/V8 isolate host
├── sandbox-wasm        Wasmtime and WASI tool host
├── sandbox-session     Session trait and backends
├── sandbox-snapshot    Snapshot, fork, and restore
├── sandbox-vmm         Firecracker driver
├── sandbox-guest       In-VM guest agent
├── sandbox-promote     Isolate-to-VM handoff
├── sandbox-policy      Capabilities, network policy, and limits
├── sandbox-trace       Event log, diff, and replay
├── sandbox-worker      Per-tenant worker process
├── sandbox-scheduler   Placement, pools, sleep, and metering
├── sandbox-server      HTTP and WebSocket API
├── sandbox-sdk         Rust client
└── sandbox-cli         Command-line client
```

See [PLAN.md](PLAN.md) for the complete milestone plan and design specification.

## Development

The repository is currently at the planning and foundation stage. Rust workspace
crates will be introduced milestone by milestone according to `PLAN.md`.

## License

Apache-2.0
