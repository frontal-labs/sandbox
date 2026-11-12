# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for significant design decisions.

## Format

Each ADR follows this template:

```markdown
# NNNN: Short Title

## Status
Proposed | Accepted | Superseded by NNNN

## Date
YYYY-MM-DD

## Context
What is the issue that we're seeing that is motivating this decision or change?

## Decision
What is the change that we're proposing and/or doing?

## Consequences
### Positive
- 

### Negative
- 

### Risks
- 

## Alternatives Considered
- 

## References
- Links to related issues, PRs, docs
```

## Index

| ID | Title | Status | Date |
|----|-------|--------|------|
| [0001](0001-protobuf-and-openapi-contracts.md) | Protobuf for internal contracts, generated OpenAPI for REST | Accepted | — |
| [0002](0002-deno-dependency-patches.md) | Patched Deno crates from frontal-labs repos | Accepted | — |
| [0003](0003-remove-unused-deno-dependencies.md) | Remove unused Deno dependencies instead of patching them | Accepted | — |
| [0004](0004-snapshot-records-live-in-the-server.md) | Snapshot records live in the server | Accepted | — |
| [0005](0005-vfs-watch-delivers-events-through-a-stream.md) | VFS watch delivers events through a stream | Accepted | — |
| [0006](0006-sleep-and-wake.md) | Sleep and wake | Accepted | — |
| [0007](0007-trace-format-and-replay.md) | Trace format and replay | Accepted | — |
| [0008](0008-durable-server-state.md) | Durable server state | Accepted | — |
| [0009](0009-server-exec-traces.md) | Server exec traces | Accepted | 2026-10-15 |
| [0010](0010-durable-live-sessions.md) | Durable live sessions | Accepted | — |
| [0011](0011-file-changes-in-traces.md) | File changes in traces | Accepted | 2026-10-19 |
| [0012](0012-filesystem-writes-in-traces.md) | Filesystem writes in traces | Accepted | 2026-10-19 |
| [0013](0013-blob-store-garbage-collection.md) | Blob store garbage collection | Accepted | 2026-10-19 |
| [0014](0014-session-limits.md) | Session limits | Accepted | 2026-10-19 |
| [0015](0015-idle-sleep-and-metering.md) | Idle sleep and metering | Accepted | 2026-10-19 |
| [0016](0016-log-structured-durable-store.md) | Log-structured durable store | Accepted | 2026-10-19 |
| [0017](0017-content-defined-chunks.md) | Content-defined chunks in a store-wide chunk area | Accepted | 2026-10-19 |
| [0018](0018-shell-recursion-limit.md) | Shell recursion limit and abort propagation | Accepted | 2026-10-19 |
| [0019](0019-shell-conformance.md) | Shell conformance against GNU bash | Accepted | 2026-10-20 |
| [0020](0020-streaming-exec.md) | Streaming exec, per finished command | Accepted | 2026-10-20 |
| [0021](0021-streaming-tool-output.md) | Streaming tool output as it is written | Accepted | 2026-10-20 |
| [0022](0022-worker-isolation.md) | Worker isolation on Linux | Accepted | 2026-10-20 |
| [0023](0023-clustered-state-on-postgres.md) | Clustered durable state on Postgres | Accepted | 2026-10-20 |
| [0024](0024-typescript-type-stripping.md) | TypeScript by type stripping | Accepted | 2026-10-20 |
| [0025](0025-warm-isolate-pool.md) | Warm isolate pool and the cold-start measurements | Accepted | 2026-10-20 |

| [0026](0026-package-layers-and-lockfile.md) | Package layers and lockfile | Accepted | 2026-10-25 |
| [0027](0027-guest-protocol-v1.md) | Guest protocol v1 | Accepted | 2026-10-25 |
| [0028](0028-per-byte-streaming-builtins.md) | Per-byte streaming for shell builtins | Accepted | 2026-11-01 |
| [0029](0029-trace-encoding-returns-errors.md) | Trace encoding returns errors | Accepted | 2026-11-03 |
| [0030](0030-websocket-exec-route.md) | WebSocket exec route | Accepted | 2026-11-03 |
| [0032](0032-websocket-cancel.md) | Cancelling a WebSocket command | Accepted | 2026-11-03 |
| [0033](0033-cpu-accounting.md) | CPU accounting for a command | Accepted | 2026-11-03 |
| [0034](0034-parked-sleeping-sessions.md) | Parked sleeping sessions | Accepted | 2026-11-05 |
| [0035](0035-object-store-blobs.md) | Blobs in an object store | Accepted | 2026-11-05 |
| [0031](0031-bazel-resolves-from-its-own-lock.md) | Bazel resolves crates from its own lock | Accepted | 2026-11-03 |
| [0036](0036-serve-config-toml.md) | TOML configuration for `serve` | Accepted | 2026-11-07 |
| [0037](0037-websocket-signals.md) | Signals on the WebSocket exec route | Accepted | 2026-11-10 |
| [0038](0038-websocket-live-stdin.md) | Live stdin on the WebSocket exec route | Accepted | 2026-11-10 |
| [0039](0039-coreutils-builtins.md) | Coreutils builtins over the VFS | Accepted | 2026-11-10 |
| [0040](0040-mcp-server.md) | MCP server over stdio | Accepted | 2026-11-10 |
| [0041](0041-session-metering.md) | Per-session CPU and memory metering | Accepted | 2026-11-11 |
| [0042](0042-billing-event-stream.md) | Durable billing event stream in Postgres | Accepted | 2026-11-11 |
| [0043](0043-pty-shell.md) | Interactive PTY shell over WebSocket | Accepted | 2026-11-11 |
| [0044](0044-pty-commands-are-checkpointed.md) | PTY commands are checkpointed, traced, and metered | Accepted | 2026-11-11 |
| [0045](0045-pty-exit-status-and-running-input.md) | PTY exit status, and typed input for running commands | Accepted | 2026-11-11 |
| [0046](0046-live-stdin-for-cat.md) | Live standard input for a top-level cat | Accepted | 2026-11-11 |
| [0047](0047-streaming-filters-and-line-input.md) | Streaming line filters, drained input for other readers, and line editing while a command runs | Accepted | 2026-11-11 |
| [0048](0048-streaming-pipelines.md) | Streaming pipelines over live input | Accepted | 2026-11-11 |
| [0049](0049-snapshot-templates.md) | Snapshot templates | Accepted | 2026-11-12 |

Add rows here as `NNNN-title.md` files are created.

## Process

1. Create new ADR file: `docs/adr/NNNN-short-title.md` (next free number, starting at 0001)
2. Fill in template
3. Submit PR for review
4. On acceptance, update status to "Accepted"
5. If superseded, link to new ADR and mark old as "Superseded"

## When to Write an ADR

- Public API changes
- Security model changes
- New crate or major refactor
- Technology choices (e.g., V8 vs WASM for Python)
- Protocol changes (guest agent, trace format)
- Performance-critical design decisions