# Architecture

> **Status:** the design below is the target architecture from
> [PLAN.md](../PLAN.md). Most components are scaffolds or stubs today. The
> "Current state" section lists what exists. Treat everything else as planned.

## Overview

Frontal Sandbox aims to provide fast, low-cost execution environments for AI
agents. A session starts in a V8-based isolate (Tier 1) and is promoted to a
real Linux microVM (Tier 2) only when a command needs one. The project is a
Rust workspace with TypeScript and Python SDKs.

## Current State

| Area | State |
|------|-------|
| `sandbox-types` | Real types: IDs, content hash (BLAKE3), errors, events, session and policy types |
| `sandbox-cas` | `BlobStore` trait, `MemStore`, `DiskStore`, `TieredStore`, with inline unit tests |
| `sandbox-vfs` | Copy-on-write overlay, Merkle directory tree with incremental commit, symlink resolution, quotas, and ops. Unit and proptest coverage |
| `sandbox-policy`, `sandbox-scheduler` | Types and logic. No tests yet |
| `sandbox-shell` | Parser, interpreter, VFS-backed builtins, variables, globs, control flow, functions, command substitution, and WASI tools from the registry or `/bin`. Subset of sh: no `case`, arithmetic, or host binaries |
| `sandbox-snapshot`, `sandbox-trace`, `sandbox-vmm`, `sandbox-guest` | Early or partial scaffolding |
| `sandbox-runtime` | `execute_script` runs JS in a fresh V8 isolate with a timeout, a heap cap, and captured `console.log`. No modules |
| `sandbox-wasm` | Runs WASI preview1 command modules. Files are the VFS directory mounted as the preopen. Stdio is in memory. Tool registry at `/etc/tools.toml`. Compiled modules cached per process |
| `sandbox-session` | `IsolateBackend` owns a VFS and a shell; `exec` and `run_script` work; `snapshot`, `sleep` return `Unsupported` |
| `sandbox-worker` | One child process per session, served as gRPC over a private Unix socket (mode 0700), with crash isolation. No seccomp, cgroups, or namespaces |
| `sandbox-proto` | Generated protobuf and gRPC bindings from `proto/`, with conversions to and from `sandbox-types` |
| `sandbox-promote` | Stub. Needs a Linux host with KVM |
| `sandbox-server` | Session, exec, and filesystem routes run against in-memory sessions behind API-key auth; snapshot and fork return 501 |
| `sdks/rust` | Client methods return `Unsupported`; embedded mode is not wired |
| `cli` | `run` and `shell` execute; other subcommands return `Unsupported` |
| `sdks/typescript`, `sdks/python` | Client code and types. Not run end to end against the server |

## System Components (target)

```
Client (Rust SDK / TypeScript SDK / Python SDK / CLI)
  -> server -> scheduler -> worker
                         -> isolate backend (shell, V8, WASM, VFS)
                         -> VM backend (Firecracker + guest agent)
  snapshot store + trace sink
```

## Core Invariants

These come from PLAN.md section 2 and the project working principles. Code
changes must preserve them.

1. **VFS is the source of truth.** Every backend mounts the same virtual
   filesystem. Promotion means "boot a VM with this VFS."
2. **Tier transparency.** The SDK should not need to know which tier it is on.
   Tier is a property of the session, exposed read-only.
3. **Capability-based access.** All host access (files, network, clock, env)
   goes through capability operations with a policy check. No ambient
   authority.
4. **Trace everything.** Every state change emits a trace event.
5. **Process isolation.** One OS process per tenant. Isolates are not a
   security boundary on their own.
6. **Session boundary.** Isolate and microVM concerns stay behind the session
   abstraction (`sandbox-session`).

## Execution Tiers (target)

### Tier 1: Isolate Backend
- V8 isolates via `deno_core` (`sandbox-runtime`)
- Shell subset interpreter (`sandbox-shell`): pipes, `&&`/`||`/`;`, redirects, and VFS builtins
- WASM tools via `wasmtime` (`sandbox-wasm`)
- Copy-on-write VFS (`sandbox-vfs`)
- Target: sub-5 ms cold start from a warm pool (not measured)

### Tier 2: VM Backend
- Firecracker microVMs (`sandbox-vmm`)
- In-VM guest agent (`sandbox-guest`)
- Full Linux compatibility
- Promotion triggered by a `NeedsVm` signal (`sandbox-promote`)
- Target: under 400 ms promotion, under 150 ms resume (not measured)

## Crate Structure

Workspace members, from the root `Cargo.toml`:

```
crates/
├── sandbox-types       # Shared IDs, hashes, errors, events, session and policy types
├── sandbox-vfs         # Copy-on-write virtual filesystem
├── sandbox-cas         # Content-addressed blob storage
├── sandbox-shell       # POSIX-ish shell and builtins
├── sandbox-runtime     # Deno/V8 isolate host
├── sandbox-wasm        # Wasmtime and WASI tool host
├── sandbox-session     # Session trait and backends
├── sandbox-snapshot    # Snapshot, fork, and restore
├── sandbox-vmm         # Firecracker driver
├── sandbox-guest       # In-VM guest agent
├── sandbox-promote     # Isolate-to-VM handoff
├── sandbox-policy      # Capabilities, network policy, limits
├── sandbox-trace       # Event log, diff, and replay
├── sandbox-worker      # Per-tenant worker process
├── sandbox-scheduler   # Placement, pools, sleep, metering
└── sandbox-server      # HTTP API (axum)
sdks/
├── rust/               # Crate `sandbox-sdk` (Rust client)
├── typescript/         # npm `sandbox-sdk` (built with Bazel and Bun)
└── python/             # Python package `sandbox_sdk`
cli/                    # Crate `cli`, binary `sandbox`
```

Dependency direction (PLAN.md section 11): `sandbox-types` and `sandbox-cas` at
the bottom, `sandbox-server` and `cli` at the top, with no cycles.

## Data Flow (target)

These flows describe the planned design. The server handlers do not yet
implement any of them.

### Session Creation
1. Client calls `POST /v1/sessions`
2. Server validates auth and quotas
3. Scheduler places the session on a worker
4. Worker spawns the isolate backend
5. Returns `SessionId` to the client

### Command Execution
1. Client calls `POST /v1/sessions/{id}/exec`
2. Server routes to the worker (gRPC is planned; `tonic` is a workspace dependency)
3. Worker executes in the session backend
4. Streams stdout and stderr as NDJSON or over WebSocket
5. Emits trace events

### Promotion (Tier 1 to Tier 2)
1. Isolate backend encounters a command that needs a real binary
2. Returns `NeedsVm { command }`
3. `sandbox-promote` pauses the isolate and commits the VFS
4. Exports the VFS to an ext4 block image
5. Acquires a VM from the warm pool and attaches the image
6. Transfers env, cwd, and limits
7. Re-runs the failed command in the VM
8. Swaps the session backend and emits a `Promoted` event

### Snapshots
1. `snapshot()` commits the VFS and serializes shell state
2. Optionally captures a V8 heap snapshot
3. Stores the result as a CAS object with metadata
4. `restore()` mounts the VFS root hash and fast-path runtime blob
5. `fork()` creates n sessions sharing a base, each with an independent overlay

## Network Architecture (target)

- Default-deny egress
- Allowlist by host pattern via `sandbox-policy`
- In-process HTTP client that enforces:
  - DNS pinning (no rebinding)
  - Blocking of private, link-local, and metadata address ranges
  - Size and time limits
  - Optional request recording

## Storage

- **CAS** (`sandbox-cas`): content-addressed blob store. `MemStore`,
  `DiskStore`, and `TieredStore` exist. Planned: zstd compression and FastCDC
  chunking; object-store backend in M5.
- **Hashing** (`sandbox-types`): `ContentHash` wraps BLAKE3.
- **VFS** (`sandbox-vfs`): layered tree (immutable base plus mutable overlay).
  Planned: Merkle hashing of the tree.
- **Snapshots** (`sandbox-snapshot`): planned as CAS objects that reference a
  VFS root and runtime state.
- **Traces** (`sandbox-trace`): planned as append-only JSONL per session.

## Observability (planned)

- **Metrics:** Prometheus (session starts, latency histograms, promotion rate,
  policy denials, OOM kills, CAS hit rate, pool depth)
- **Logs:** structured JSON with request ID and session ID. The server has a
  request-ID middleware stub (`crates/sandbox-server/src/middleware.rs`).
- **Traces:** OpenTelemetry spans across server, scheduler, and worker

## Security Assumptions

- The server has API-key auth, but no key provisioning, TLS, or rate limiting, and snapshot and fork are not implemented. Do not expose it.
- See [SECURITY_MODEL.md](SECURITY_MODEL.md) for the model under design.
