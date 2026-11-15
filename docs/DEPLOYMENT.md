# Deployment Guide

**Status: a production deployment path exists.** The server is a single
`sandbox serve` binary (in the `cli` crate) that runs the server, scheduler,
and worker in one process. It supports a TOML config file, a durable data dir,
Postgres-backed metadata (behind the `clustered` feature), an S3-backed CAS
tier, and a `/metrics` endpoint on the main HTTP port. This page separates
what runs today from what is still planned.

## Topology

There is **one** public server role: `sandbox serve`. The codebase ships a
single binary; there is no standalone scheduler or worker subcommand today
(only a hidden Unix-socket-internal `Worker` and `ScriptRun`). Do not
provision separate scheduler/worker units or K8s Deployments — run one
`sandbox serve` process per node.

| Surface | Port | Notes |
|---------|------|-------|
| HTTP API (`/v1/...`) | 8080 (default, `server.addr`) | Requires an API key. No TLS built in; front with a terminating LB/proxy. |
| `/metrics` | **8080** (same listener) | Prometheus text format. No separate 9090 listener exists; the config's `telemetry.metrics_addr` field is accepted but not consumed. |
| `/v1/openapi.json` | 8080 | Public OpenAPI document (no auth); useful as a liveness probe. |

## What exists today

### Build

```bash
# Rust workspace (crates/*, sdks/rust, cli)
cargo build --workspace
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.95).

The TypeScript SDK (`sdks/typescript`, npm package `@frontal-labs/sandbox-sdk`)
and the Python SDK (`sdks/python`) are built separately. The root `package.json`
scripts `ts:build`, `ts:test`, and `ts:lint` run through Bazel (`BUILD.bazel`,
`MODULE.bazel`).

The Postgres-backed state store is behind the `clustered` feature
(`cli/Cargo.toml`: `clustered = ["sandbox-server/clustered"]`). A build without
it refuses `--database-url`. The Dockerfile builds with default features, so a
clustered image needs `--features clustered` added to its `cargo build` step.

### CLI

The `sandbox` (a legacy alias of `frontal-sandbox`) binary is built from the
`cli` crate (`cli/src/main.rs`). `run` and `shell` execute builtin commands in
an in-process isolate session. `record --trace`, `replay`, and `bench` work on
the local runtime. `snapshot` and `fork` call a server named by
`SANDBOX_BASE_URL`, and `serve` starts it in the foreground
(`cli/src/commands.rs`).

```bash
cargo run -p cli -- run "echo hi"   # prints "hi"
```

### Server

`crates/sandbox-server` defines routes in `src/api.rs`. Every `/v1` route
except `/v1/openapi.json` requires a valid API key. The server runs through the
CLI:

```bash
sandbox serve --addr <addr> [--data-dir <dir>] [--config <path>] [--database-url <dsn>]
```

- **Config file loader** (`crates/sandbox-server/src/config.rs`): `--config`
  loads a TOML file; CLI flags override mapped fields; `SANDBOX_*` and OTLP env
  vars override file values. All sub-tables are optional (`required: []` in the
  JSON schema), so a minimal `[server]`-only file loads cleanly.
- **API key**: generated on first start and printed once to stderr, then stored
  as a BLAKE3 digest in the data dir. There is no env var to inject a key.
- **Persistence**: with `--data-dir`, keys (as digests), snapshots, and sleeping
  sessions persist across restarts. Without it, state is in memory and a new key
  is printed each start.
- **Metrics**: `/metrics` on the main 8080 listener (`api.rs`).
- **Postgres**: `--database-url` enables the clustered (Postgres) metadata
  backend (`clustered` feature).
- **CAS tier**: S3 via `object_store` (reads `AWS_ACCESS_KEY_ID` /
  `AWS_SECRET_ACCESS_KEY` from the environment), with a local disk cache in
  front (`TieredStore`).
- **TLS**: none built in. Do not expose the server to untrusted networks
  without a terminating TLS proxy in front.

### Container image

`docker/` holds the container setup: a two-stage `Dockerfile` (Rust 1.95
builder, `debian:bookworm-slim` runtime, non-root, tini) and `compose.yaml` with
a named state volume, a loopback-only port, a health check, and hardening.
`smoke-test.sh` checks auth, session creation, and restart persistence. See
[docker/README.md](../docker/README.md).

`infrastructure/docker/` overlays the base for staging and prod:
`compose.staging.yaml` and `compose.prod.yaml` add TLS termination, rate
limiting, and separate named volumes. Deploy with:

```bash
docker compose -f docker/compose.yaml -f infrastructure/docker/compose.prod.yaml up -d
```

### Infrastructure tree

- `infrastructure/terraform/` — AWS (reference) Terraform: KVM-capable hosts,
  S3 CAS bucket, security groups (SSH 22, API+metrics 8080), and an optional
  NLB. `terraform validate` passes.
- `infrastructure/kubernetes/manifests.yaml` — EKS reference: one Deployment of
  the single `sandbox serve` process, ConfigMap, Secrets, Service (8080 only),
  and Ingress.
- `infrastructure/systemd/` — `sandbox-server.service` + `sandbox.target`
  (single-serve; no separate scheduler/worker units).
- `infrastructure/config/` — JSON schema + `defaults.toml` + env examples
  (`dev.toml`, `staging.toml`, `production.toml`).
- `infrastructure/monitoring/` — Prometheus scrape config, alerting/recording
  rules, and a Grafana dashboard.
- `infrastructure/scripts/` — `bootstrap.sh`, `deploy.sh`, `smoke.sh`,
  `backup.sh`, `restore.sh`, `upgrade.sh`.

### Continuous integration and releases

| File | What it does | Status |
|------|--------------|--------|
| `.github/workflows/ci.yml` | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `cargo bench --no-run`, using the `rust-toolchain.toml` pin | Not verified to pass in this refresh. |
| `.github/workflows/bazel.yml` | `bazel build //...` and `bazel test //...` with Bazelisk | Active |
| `.github/workflows/pr-checks.yml` | Conventional-commit PR title, semver label, size warning, auto-assign | Active |
| `.github/workflows/benchmarks.yml` | Criterion and load-test runs, baseline comparison | Runs, but see [PERFORMANCE.md](PERFORMANCE.md). `benchmarks/baselines/` is empty, so no regression check compares against recorded data. |
| `.github/workflows/release-binaries.yml` | Tag build of the `cli` crate (binary `sandbox`) for five targets, npm platform packages and the `@frontal-labs/sandbox-cli` wrapper to npmjs, `SHA256SUMS` attached to the release | Builds `-p cli`. Windows copies `sandbox.exe`. |
| `.github/workflows/codeql-analysis.yml` | CodeQL analysis | Not reviewed in this refresh |
| `.github/workflows/infrastructure.yml` | Validates the `infrastructure/` tree: Terraform `tofu validate` + TOML examples against the JSON schemas | Runs on `main` push/PR touching `infrastructure/**`. |

`.devcontainer/Dockerfile` builds a development container (Rust and Bun). It is
not a deployment image.

### Files this page used to reference that do not exist

- `deploy/` (now consolidated into `infrastructure/` + `docker/`)
- `tools/` and `images/` (rootfs build scripts and Tier 2 images)
- `Dockerfile.server` and a `sandbox-server` binary target (the server runs via `sandbox serve`)
- `sandbox-admin` binary, `sandbox-bench-*` binaries, and the `bench/load/*.js` k6 scripts
- `crates/sandbox-scheduler/migrations/` (the scheduler has no SQL migrations)

## Requirements

- **Tier 1 (isolate)**: runs builtin commands over the VFS (`sandbox-session`,
  `sandbox-shell`) in the V8 runtime. No snapshot or sleep at the isolate level.
- **Tier 2 (QEMU)**: x86_64 or aarch64 Linux. QEMU is required on the host;
  `/dev/kvm` is needed for hardware acceleration (`Accel::Kvm`), and software
  emulation (`Accel::Tcg`) works without it, at software speed.
- **Capacity**: the project specification section 6 targets 10,000+ idle
  sessions per 64 GB host. This is a target, not a measured result.

## Single node

A single static binary with a systemd unit, a container image, and Terraform
for cloud hosts. The systemd unit (`infrastructure/systemd/sandbox-server.service`),
the container image (`docker/` + `infrastructure/docker/`), and the Terraform
(`infrastructure/terraform/`) all exist. The config file loader and config
schema are implemented.

## Clustered

The project specification section 4.14 and milestone M5 call for:

- Scheduler metadata in Postgres via `sqlx`, behind a `Meta` trait — implemented
  behind the `clustered` feature (`--database-url`).
- CAS on an S3-compatible object store via `object_store`, with a local cache
  in front (`TieredStore`) — implemented; reads standard AWS env vars.
- K8s reference manifests in `infrastructure/kubernetes/manifests.yaml` — one
  Deployment of the single serve process.

## Tier 2 (QEMU)

Implemented:

- QEMU driven directly (`crates/sandbox-vmm/src/qemu.rs`): a spawned process
  with a kernel and initrd, the guest agent as init, the agent reachable on a
  forwarded loopback port.
- Warm pool of pre-booted guests (`crates/sandbox-vmm/src/pool.rs`).
- Promotion from Tier 1 (`crates/sandbox-promote/src/promote.rs`), wired into
  the server for sessions created with the `vm` tier preference.
- Snapshot/restore through the QEMU monitor (`savevm` / `loadvm`).

Not present yet:

- Base and browser rootfs images built by scripts in `tools/` and `images/`
  (the emulated test image in `docker/emulated-vm/` exists).
- Automatic re-promotion of an existing isolate session when a command fails
  with `NeedsVm`.

## Operations

- **Metrics**: `/metrics` on 8080. Metric names are defined in
  `crates/sandbox-server/src/metrics.rs`; Prometheus scrape config is in
  `infrastructure/monitoring/prometheus.yaml`.
- **Logs**: structured JSON via `tracing`; the server middleware logs request
  method and URI.
- **Traces**: OpenTelemetry via `server.telemetry.otlp_endpoint` in config
  (optional; mutually exclusive with the clustered Postgres path today).
- **Backup and recovery**: `infrastructure/scripts/backup.sh` and
  `restore.sh` cover CAS + metadata backup/restore.
- **Upgrades**: `infrastructure/scripts/upgrade.sh` rolls a single node.

## Security

Deployment security depends on the worker and VMM layers. Read
[SECURITY_MODEL.md](SECURITY_MODEL.md) before any deployment. In particular:

- Do not expose the API without TLS. Authentication exists (API keys, see
  `auth.rs`), but there is no built-in TLS or rate limiting — front with a
  terminating proxy.
- Do not put real credentials in session env or secrets. The secrets proxy is
  not implemented.
- The worker needs `/dev/kvm` access for Tier 2. Plan the privilege model with
  the security review in the project specification section 5.
- Secrets (Postgres DSN password, S3 credentials) are injected from a
  Kubernetes Secret or a secrets manager, not baked into the manifest. The API
  key is self-generated on first start — it is not a boot-time secret.
