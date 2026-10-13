# sandbox-wasm

WASI tool host on `wasmtime` for running WebAssembly tools in the sandbox.

## Status

Stub. `WasiHost::new()` builds a wasmtime `Engine` and links WASI preview 1 into a `Linker`. `WasiHost::run_wasm` returns `SandboxError::unsupported` (a `TODO` in `src/wasi.rs`). No module loading, instance execution, VFS-backed filesystem, limits, tool registry (`tools.toml`), or AOT caching exists yet. No tests exist.

## Overview

Intended to be a WASI host for tools, with a filesystem backed by `sandbox-vfs` and no host preopens. Source: `crates/sandbox-wasm/src/`.

## Public API (from `src/lib.rs`)

- `WasiHost` (`wasi.rs`): `new() -> Result<Self, SandboxError>`, `run_wasm(&self, &[u8]) -> Result<(), SandboxError>` (stub), `Default`.
- `WasiState { ctx: WasiP1Ctx }` (`wasi.rs`): state type for the linker.

## Invariants and security assumptions

- Target: no host directories are preopened; the only filesystem is the VFS. Not enforced yet, because no WASI filesystem is wired up.
- Target limits: memory via `StoreLimits`, CPU via `epoch_interruption`, wall time via an async timeout. None are implemented.

## Example

Omitted: `run_wasm` is a stub.

See `PLAN.md` section 4.6 for the target spec.
