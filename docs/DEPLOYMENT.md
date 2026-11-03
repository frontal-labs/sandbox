# Deployment Guide

**Status: no production deployment path exists yet.** The server, scheduler, worker, and VM layers are scaffolds or stubs (see [ARCHITECTURE.md](ARCHITECTURE.md) and [MILESTONES.md](MILESTONES.md)). This page separates what runs today from what is planned. Planned sections describe intent from the project specification sections 4.14, 4.15, and 10. They are not runnable instructions.

## What exists today

### Build

```bash
# Rust workspace (18 members: crates/*, sdks/rust, cli)
cargo build --workspace
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.95).

The TypeScript SDK (`sdks/typescript`, npm package `@frontal-labs/sandbox-sdk`) and the Python SDK (`sdks/python`) are built separately. The root `package.json` scripts `ts:build`, `ts:test`, and `ts:lint` run through Bazel (`BUILD.bazel`, `MODULE.bazel`).

### CLI

The `frontal-sandbox` binary is built from the `cli` crate (`cli/src/main.rs`). `run` and `shell` execute builtin commands in an in-process isolate session. `record` with `--trace`, `replay`, and `bench` work on the local runtime. `snapshot` and `fork` call a server named by `SANDBOX_BASE_URL`, and `serve` starts it in the foreground (`cli/src/commands.rs`).

```bash
cargo run -p cli -- run "echo hi"   # prints "hi"
```

### Server

`crates/sandbox-server` defines routes in `src/api.rs`, and every `/v1` route except `/v1/openapi.json` requires a valid API key. The server runs through the CLI: `frontal-sandbox serve --addr <addr> [--data-dir <dir>]` (`cli/src/commands.rs`). With `--data-dir`, keys, snapshots, and sleeping sessions persist across restarts. Without it, state is in memory and a new key is printed on each start. The first durable start prints one API key to stderr. There is no config file loader, no separate health endpoint (`/v1/openapi.json` serves as the liveness probe), and no metrics endpoint. Do not expose the server to untrusted networks: it has no TLS and no rate limiting.

### Container image

`docker/` holds a container setup for local and development use: a two-stage `Dockerfile` (Rust 1.95 builder, `debian:bookworm-slim` runtime, non-root, tini), `compose.yaml` with a named state volume, a loopback-only port, a health check, and hardening, and `smoke-test.sh`, which checks health, auth, session creation, and restart persistence. See [docker/README.md](../docker/README.md). The image is not a production deployment: it has no TLS or rate limiting, and it needs network access at build time to fetch the prebuilt V8 archive used by `deno_core`.

### Continuous integration and releases

| File | What it does | Status |
|------|--------------|--------|
| `.github/workflows/ci.yml` | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `cargo bench --no-run`, using the `rust-toolchain.toml` pin | Runs without Nix. Not verified to pass in this refresh. |
| `.github/workflows/pr-checks.yml` | Conventional-commit PR title, semver label, size warning, auto-assign | Active |
| `.github/workflows/benchmarks.yml` | Criterion and load-test runs, baseline comparison | Runs, but see [PERFORMANCE.md](PERFORMANCE.md). `benchmarks/baselines/` is empty, so no regression check compares against recorded data. |
| `.github/workflows/release-binaries.yml` | Tag build of the `cli` crate (binary `frontal-sandbox`) for five targets, npm platform packages and the `@frontal-labs/sandbox-cli` wrapper to npmjs, `SHA256SUMS` attached to the release | Builds `-p cli`. Windows copies `frontal-sandbox.exe`. |
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

These are the targets from the project specification. Versions are not pinned in the repository.

- **Tier 1 (isolate)**: no host requirements are defined yet. The isolate backend runs builtin commands over the VFS (`sandbox-session`, `sandbox-shell`). The V8 runtime and WASM tools are not wired in, and there is no snapshot or sleep.
- **Tier 2 (Firecracker)**: x86_64 or aarch64 Linux with `/dev/kvm`, cgroups v2, and user namespaces. Firecracker version is to be chosen in milestone M4. The previous "Firecracker 1.8+" note is not verified by this repository.
- **Capacity**: the project specification section 6 targets 10,000+ idle sessions per 64 GB host. This is a target, not a measured result.

## Planned: single node

The project specification section 10 calls for a single static binary with a systemd unit, a container image, and Terraform for cloud hosts. A local container image exists (see [Container image](#container-image)). The systemd unit and Terraform do not exist. Config schema, the worker config file, and the admin CLI are not designed yet.

## Planned: clustered

The project specification section 4.14 and milestone M5 call for:

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

- **Metrics**: the project specification section 10 lists session starts, start latency, exec latency, promotion rate, policy denials, OOM kills, snapshot size, CAS hit rate, and pool depth. No metric names are defined in code yet.
- **Logs**: structured JSON with request ID and session ID. The `tracing` setup in `crates/sandbox-server/src/middleware.rs` logs request method and URI only.
- **Traces**: OpenTelemetry spans across server, scheduler, and worker. Not implemented.
- **Backup and recovery**: CAS and metadata backup, GC, and snapshot verification. Not implemented. The existing `sandbox-admin` commands do not exist.
- **Upgrades**: blue-green worker rollout. Not implemented; no drain or admin command exists.
- **Scaling and alerting thresholds**: none are defined in code or in the project specification. Define them alongside the metrics.

## Security

Deployment security depends on the worker and VMM layers, which are not implemented. Read [SECURITY_MODEL.md](SECURITY_MODEL.md) before any deployment. In particular:

- Do not expose the API. Authentication exists (API keys, see `auth.rs`), but there is no key provisioning, rate limiting, or TLS yet.
- Do not put real credentials in session env or secrets. The secrets proxy is not implemented.
- The worker is planned to need `/dev/kvm` access for Tier 2. Plan the privilege model with the security review in the project specification section 5.
