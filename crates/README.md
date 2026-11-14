# Crates

The core Rust crates that make up the Frontal Sandbox system. This is a Cargo workspace with additional Bazel build support.

## Workspace Structure

```
crates/
├── sandbox-cas/        # Content-addressed blob store (MemStore, DiskStore, TieredStore)
├── sandbox-vfs/        # Virtual filesystem with copy-on-write and Merkle commits
├── sandbox-session/    # Session trait and isolate (Tier 1) backend
├── sandbox-promote/    # Tier 1 to Tier 2 promotion
├── sandbox-proto/      # Generated protobuf + gRPC bindings and conversions
├── sandbox-server/     # HTTP API server (axum) with OpenAPI generation
├── sandbox-worker/     # Worker process for process-isolated execution
├── sandbox-snapshot/   # Snapshot, fork, and restore (early, in-memory)
├── sandbox-vmm/        # QEMU VMM driver for Tier 2
├── sandbox-guest/      # In-VM guest agent for Tier 2
├── sandbox-shell/      # Built-in shell for isolate sessions
├── sandbox-types/      # Shared domain types (internal, not published)
└── sandbox-wasm/       # WASI tools registry and conformance tests
```

## Key Crates

| Crate | Purpose | Status |
|-------|---------|--------|
| `sandbox-cas` | Blob storage with BLAKE3 hashing, zstd compression, chunking | Partial |
| `sandbox-vfs` | Copy-on-write VFS with Merkle commits, quotas, watches | Working |
| `sandbox-session` | `Session` trait, `IsolateBackend`, checkpoints | Partial |
| `sandbox-proto` | Protobuf/gRPC codegen + conversions | Working |
| `sandbox-server` | REST API server, auth, limits, OpenAPI | Early |
| `sandbox-worker` | Process-isolated session worker | Working (no sandboxing) |
| `sandbox-snapshot` | Snapshot/fork/restore | Early |
| `sandbox-vmm` | QEMU driver, warm VM pool | Working (needs QEMU) |
| `sandbox-guest` | In-VM agent (gRPC over a forwarded port) | Working |

## Building

```bash
# Cargo (from workspace root)
cargo build --workspace
cargo test --workspace

# Bazel
bazel build //crates/...
bazel test //crates/...
```

## Conformance

The `sandbox-wasm` crate runs conformance tests for the WASI tools (`jq`, `rg`) against reference implementations.

## License

Apache-2.0