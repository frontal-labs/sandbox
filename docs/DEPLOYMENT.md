# Deployment Guide

**Status: no production deployment path exists yet.** The server, scheduler, worker, and VM layers are scaffolds or stubs (see [ARCHITECTURE.md](ARCHITECTURE.md) and [MILESTONES.md](MILESTONES.md)). This page separates what runs today from what is planned. Planned sections describe intent from PLAN.md sections 4.14, 4.15, and 10. They are not runnable instructions.

## What exists today

### Build

```bash
# Rust workspace (18 members: crates/*, sdks/rust, cli)
cargo build --workspace
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.95).

The TypeScript SDK (`sdks/typescript`, npm package `sandbox-sdk`) and the Python SDK (`sdks/python`) are built separately. The root `package.json` scripts `ts:build`, `ts:test`, and `ts:lint` run through Bazel (`BUILD.bazel`, `MODULE.bazel`).

### CLI

The `sandbox` binary is built from the `cli` crate (`cli/src/main.rs`). `run` and `shell` execute builtin commands in an in-process isolate session. `snapshot`, `fork`, `replay`, `serve`, and `bench` return "not implemented" errors (`cli/src/commands.rs`).

```bash
cargo run -p cli -- run "echo hi"   # prints "hi"
```

### Server

`crates/sandbox-server` defines routes in `src/api.rs`, and every `/v1` route requires a valid API key. The handlers are placeholders that answer `501 Not Implemented`. There is no server `main` in the repository, no key-provisioning path, no config file loader, no health endpoint, and no metrics endpoint. Do not expose a server built from this code.

### Continuous integration and releases

| File | What it does | Status |
|------|--------------|--------|
| `.github/workflows/ci.yml` | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `cargo bench --no-run`, using the `rust-toolchain.toml` pin | Runs without Nix. Not verified to pass in this refresh. |
| `.github/workflows/pr-checks.yml` | Conventional-commit PR title, semver label, size warning, auto-assign | Active |
| `.github/workflows/benchmarks.yml` | Criterion and load-test runs, baseline comparison | Runs, but see [PERFORMANCE.md](PERFORMANCE.md). `benchmarks/baselines/` is empty, so no regression check compares against recorded data. |
| `.github/workflows/release.yml` | Tag build with `cargo build --release --workspace`, attaches `target/release/sandbox*` to a GitHub release | Runs without Nix. Not verified in this refresh. |
| `.github/workflows/release-binaries.yml` | Tag build of the `cli` crate (binary `sandbox`) for five targets, npm platform packages to the GitHub npm registry, attaches to release | Builds `-p cli`. Windows copies `sandbox.exe`. Not verified in this refresh. |
| `.github/workflows/update-flake.yml` | Weekly `nix flake update` PR | Deleted (staged). The flake it updated was removed. |
| `.github/workflows/codeql-analysis.yml` | CodeQL analysis | Not reviewed in this refresh |

`.devcontainer/Dockerfile` builds a development container (Rust and Bun). It is not a deployment image.

### Files this page used to reference that do not exist

- `deploy/` (Dockerfile, Terraform, systemd units, Helm chart, Grafana dashboards)
- `tools/` and `images/` (rootfs build scripts and Tier 2 images)
- `Dockerfile.server`, `config.toml` examples, and a `sandbox-server` binary target
- `sandbox-admin` binary, `sandbox-bench-*` binaries, and the `bench/load/*.js` k6 scripts
- `crates/sandbox-scheduler/migrations/` (the scheduler has no SQL migrations)
- `/health` and `/admin/workers` endpoints
- `sandbox_*` Prometheus metrics (no metrics are exported by code)
- The `nix develop` workflow and `flake.nix` (CI no longer uses them)

## Requirements (planned)

These are the targets from PLAN.md. Versions are not pinned in the repository.

- **Tier 1 (isolate)**: no host requirements are defined yet. The isolate backend runs builtin commands over the VFS (`sandbox-session`, `sandbox-shell`). The V8 runtime and WASM tools are not wired in, and there is no snapshot or sleep.
- **Tier 2 (Firecracker)**: x86_64 or aarch64 Linux with `/dev/kvm`, cgroups v2, and user namespaces. Firecracker version is to be chosen in milestone M4. The previous "Firecracker 1.8+" note is not verified by this repository.
- **Capacity**: PLAN.md section 6 targets 10,000+ idle sessions per 64 GB host. This is a target, not a measured result.

## Planned: single node

PLAN.md section 10 calls for a single static binary with a systemd unit, a container image, and Terraform for cloud hosts. None of these exist. Config schema, the worker config file, and the admin CLI are not designed yet.

## Planned: clustered

PLAN.md section 4.14 and milestone M5 call for:

- Scheduler metadata in Postgres via `sqlx`, behind a `Meta` trait. Single-node metadata would use `redb` or SQLite. Neither is implemented.
- CAS on an S3-compatible object store via `object_store`, with a local cache in front (`TieredStore`). The `ObjectStore` implementation is not present. `TieredStore` exists in `crates/sandbox-cas/src/tiered_store.rs`; check its backend before relying on it.
- Trace sink: JSONL locally, with an optional Parquet or ClickHouse sink. Not implemented.

## Planned: Tier 2 (Firecracker)

Planned pieces, none implemented:

- Firecracker driven over its Unix-socket REST API, with a jailer per VM (`crates/sandbox-vmm/src/jailer.rs` returns an unsupported error).
- Base and browser rootfs images built by scripts in `tools/` and `images/` (not present).
- Warm VM pool per image (`crates/sandbox-vmm/src/pool.rs`, scaffold only).
- Promotion from Tier 1 (`crates/sandbox-promote` returns an unsupported error).

## Planned: operations

These are design notes, not current behavior.

- **Metrics**: PLAN.md section 10 lists session starts, start latency, exec latency, promotion rate, policy denials, OOM kills, snapshot size, CAS hit rate, and pool depth. No metric names are defined in code yet.
- **Logs**: structured JSON with request ID and session ID. The `tracing` setup in `crates/sandbox-server/src/middleware.rs` logs request method and URI only.
- **Traces**: OpenTelemetry spans across server, scheduler, and worker. Not implemented.
- **Backup and recovery**: CAS and metadata backup, GC, and snapshot verification. Not implemented. The existing `sandbox-admin` commands do not exist.
- **Upgrades**: blue-green worker rollout. Not implemented; no drain or admin command exists.
- **Scaling and alerting thresholds**: none are defined in code or in PLAN.md. Define them alongside the metrics.

## Security

Deployment security depends on the worker and VMM layers, which are not implemented. Read [SECURITY_MODEL.md](SECURITY_MODEL.md) before any deployment. In particular:

- Do not expose the API. Authentication exists (API keys, see `auth.rs`), but there is no key provisioning, rate limiting, or TLS yet.
- Do not put real credentials in session env or secrets. The secrets proxy is not implemented.
- The worker is planned to need `/dev/kvm` access for Tier 2. Plan the privilege model with the security review in PLAN.md section 5.
