# Infrastructure Plan — `infrastructure/`

> Status: proposed. This directory does not exist yet. It is the home for
> deployment, configuration, provisioning, and operations material for the
> Frontal Sandbox server, scheduler, and worker. Nothing here is runnable
> until the crates it references (`sandbox-server`, `sandbox-scheduler`,
> `sandbox-worker`) expose the configuration and CLI surface this plan
> specifies.

## 1. Goals

Provide a complete, milestone-aligned path from "works on a laptop" to
"runs on a KVM-capable cloud host":

- **Single node:** one static `sandbox` binary (frontal-sandbox is a legacy alias) + systemd units, a container
  image, and a TOML config file with sane defaults.
- **Cloud (bare metal / KVM instances):** OpenTofu (Terraform-compatible)
  provisioning of compute with `/dev/kvm`, cgroups v2, security groups, and
  an S3-compatible object-store bucket for the CAS tier.
- **Clustered (M5):** Kubernetes manifests for the scheduler + workers,
  Postgres metadata store, object-store CAS, Prometheus metrics, and Grafana
  dashboards.
- **Operations:** bootstrap, deploy, blue-green upgrade, backup, and restore
  scripts; alerting rules; runbooks.

All of this is driven by a versioned config schema that the Rust server
crate loads at startup (see section 4), so operators configure behavior in
one place instead of editing flags.

## 2. Non-goals (v1)

- Windows or macOS hosts. Tier 2 requires Linux with `/dev/kvm`.
- Multi-region replication (PLAN.md non-goal).
- GPU workloads.
- A managed control plane. The operator provisions hosts and runs the binary
  themselves.

## 3. Directory structure

```
infrastructure/
├── PLAN.md              # this document
├── README.md            # purpose, quick start, conventions, index
├── adr/                 # infrastructure ADRs (numbered, mirrors docs/adr/)
│   ├── README.md
│   ├── 0001-config-schema-and-loading.md
│   ├── 0002-deployment-targets.md
│   └── ...
├── config/              # configuration schema, defaults, examples
│   ├── schema/          # JSON Schema for each service config
│   │   ├── server.schema.json
│   │   ├── scheduler.schema.json
│   │   └── worker.schema.json
│   ├── defaults.toml    # built-in defaults (embedded by the server crate)
│   └── examples/        # per-environment operator examples
│       ├── dev.toml
│       ├── staging.toml
│       └── production.toml
├── systemd/             # single-node units (installed by scripts/deploy.sh)
│   ├── sandbox-server.service
│   ├── sandbox-scheduler.service
│   ├── sandbox-worker@.service   # template, one per tenant worker
│   └── sandbox.target
├── terraform/           # OpenTofu/Terraform for KVM-capable cloud hosts
│   ├── main.tf
│   ├── providers.tf
│   ├── versions.tf
│   ├── variables.tf
│   ├── outputs.tf
│   ├── compute.tf       # instances with /dev/kvm + cgroups v2
│   ├── networking.tf    # VPC, subnets, security groups
│   ├── storage.tf       # object-store bucket (CAS tier) + block volumes
│   └── firewall.tf      # ingress rules (8080, 9090 metrics, 22 SSH)
├── docker/              # production overlay for the local stack
│   └── compose.prod.yaml   # extends docker/compose.yaml (TLS, rate limit, metrics)
├── kubernetes/          # clustered deployment (M5)
│   ├── namespace.yaml
│   ├── configmap.yaml
│   ├── secret.yaml
│   ├── server-deployment.yaml
│   ├── scheduler-deployment.yaml
│   ├── worker-daemonset.yaml   # one worker per KVM node
│   ├── service.yaml
│   ├── ingress.yaml
│   ├── hpa.yaml
│   └── pod-disruption-budget.yaml
├── monitoring/          # Prometheus + Grafana (M5)
│   ├── prometheus.yaml
│   ├── alerting-rules.yaml
│   ├── recording-rules.yaml
│   └── grafana/             # dashboards (JSON)
├── scripts/             # operator helpers
│   ├── bootstrap.sh     # provision a host (deps, KVM, cgroups, firewall)
│   ├── deploy.sh        # install binary + systemd units + config
│   ├── upgrade.sh       # blue-green worker rollout
│   ├── backup.sh        # CAS + metadata backup
│   ├── restore.sh       # restore from backup
│   └── smoke.sh         # end-to-end health check against a live endpoint
└── ci/                  # CI glue for infrastructure changes
    └── tf-plan.yml      # OpenTofu plan on PRs touching infrastructure/
```

## 4. Configuration design

### 4.1 Format

TOML, loaded from `--config <path>` (default `/etc/sandbox/config.toml`)
with `SANDBOX_*` environment-variable overrides (see section 4.4). TOML is
chosen because it is operator-friendly, supports comments, and parses cleanly
with the `toml` crate already implied by the workspace's serde usage.

### 4.2 Schema

Three service configs, each with a JSON Schema file under
`infrastructure/config/schema/` and a Rust `Config` struct deriving
`Deserialize` + `JsonSchema` (schemars is already a workspace dependency).

`server` config:
- `addr` (string, default `0.0.0.0:8080`)
- `data_dir` (string, default `/var/lib/sandbox`)
- `auth` { `api_key_hash` (string, optional), `rate_limit_rps` (u32) }
- `tls` { `enabled` (bool), `cert_file`, `key_file` }
- `telemetry` { `metrics_addr` (string, default `0.0.0.0:9090`),
  `otlp_endpoint` (string, optional), `service_name` (string, default `sandbox-server`) }
- `scheduler` { `address` (string, default `127.0.0.1:50051`) }

`scheduler` config:
- `addr` (string, default `0.0.0.0:50051`)
- `metadata` { `backend` (`redb` | `postgres`), `dsn` (string) }
- `cas` { `backend` (`disk` | `tiered` | `object`), `path` (string), `bucket` (string), `cache_path` (string) }
- `pools` { `isolate_warm_pool_size` (u32), `vm_warm_pool_size` (u32) }
- `idle` { `sleep_after_seconds` (u64) }
- `metering` { `enabled` (bool), `export_interval_seconds` (u64) }

`worker` config:
- `addr` (string, default `127.0.0.1:50052`)
- `tenant_id` (string)
- `scheduler_addr` (string)
- `isolation` { `seccomp` (bool), `cgroups_v2` (bool), `namespaces` (bool) }
- `resources` { `memory_limit_bytes` (u64), `cpu_limit_ms` (u64), `pids_limit` (u32) }
- `tier2` { `enabled` (bool), `kvm_device` (string, default `/dev/kvm`),
  `firecracker_binary` (string), `socket_dir` (string) }

### 4.3 Loading precedence (highest wins)

1. `SANDBOX_<SECTION>_<KEY>` environment variables (e.g. `SANDBOX_SERVER_ADDR`)
2. `--config` TOML file (default `/etc/sandbox/config.toml`)
3. `defaults.toml` compiled into the binary
4. Hardcoded Rust defaults

### 4.4 Integration point

`crates/sandbox-server` gains a `config` module exposing:

```rust
pub struct ServerConfig { ... }   // serde + schemars
pub fn load(path: Option<&Path>) -> Result<ServerConfig, ConfigError>
```

The CLI `serve` subcommand accepts `--config`. The same pattern is repeated
in `sandbox-scheduler` and `sandbox-worker`.

## 5. Deployment targets

### 5.1 Single node (M3)

- Host: any Linux, x86_64 or aarch64. Tier 1 only (isolate); Tier 2 needs
  `/dev/kvm`.
- Install: `scripts/bootstrap.sh` then `scripts/deploy.sh`.
- Run: `systemctl start sandbox-server`; workers via
  `systemctl start sandbox-worker@<tenant>`.
- Data: `/var/lib/sandbox` (keys as digests, snapshots, sleeping sessions).

### 5.2 Cloud host (M4+)

- OpenTofu provisions one or more KVM-capable instances (`compute.tf`),
  security groups allowing `8080` (API), `9090` (metrics), and SSH (`firewall.tf`),
  an object-store bucket for the CAS tier (`storage.tf`), and block volumes
  for per-VM overlay images.
- The instance runs `sandbox-server` + `sandbox-worker` under systemd.

### 5.3 Clustered (M5)

- Kubernetes manifests under `kubernetes/`. One `worker` DaemonSet pod per
  KVM node (needs privileged/securityContext for `/dev/kvm` and device
  plugin). `server` and `scheduler` are Deployments behind a Service.
- Metadata in Postgres; CAS on object store; metrics scraped by Prometheus.

## 6. Observability

### 6.1 Metrics (Prometheus)

Defined in `monitoring/prometheus.yaml` with scrape configs for
`server`, `scheduler`, and `worker` metric endpoints (`/metrics`). Recording
and alerting rules cover:

- `sandbox_session_starts_total`, `sandbox_session_start_latency_seconds`
- `sandbox_exec_duration_seconds`, `sandbox_promotion_total`,
  `sandbox_policy_denials_total`, `sandbox_oom_kills_total`,
- `sandbox_cas_hit_rate`, `sandbox_pool_depth`, `sandbox_idle_sessions`,
- `sandbox_cpu_ms_total`, `sandbox_memory_gb_seconds`

### 6.2 Alerts (`monitoring/alerting-rules.yaml`)

- SandboxDown (server scrape failing for 2m)
- SandboxHighOOMKills (> 5/min)
- SandboxPoolExhausted (warm pool at 0 for 5m)
- SandboxDiskNearlyFull (state volume > 85%)

### 6.3 Dashboards (`monitoring/grafana/`)

One dashboard: sessions, exec latency, promotion rate, CAS hit rate, pool
depth, and host resource usage.

## 7. Scripts

| Script | Purpose |
|--------|---------|
| `bootstrap.sh` | Install toolchain deps, enable KVM/cgroups v2, harden sshd, open firewall ports |
| `deploy.sh` | Copy the static binary to `/usr/local/bin/sandbox` (frontal-sandbox is a legacy alias), install systemd units, write `/etc/sandbox/config.toml`, start services |
| `upgrade.sh` | Blue-green worker rollout: drain, stop old units, start new binary, verify health |
| `backup.sh` | Snapshot `/var/lib/sandbox`, dump Postgres (clustered), upload to object store |
| `restore.sh` | Download backup, restore state + metadata, restart services |
| `smoke.sh` | `curl /v1/openapi.json`, create a session, exec `echo hi`, destroy |

## 8. CI integration

Add `.github/workflows/infrastructure.yml` (or fold into `ci.yml`) that, on
PRs touching `infrastructure/**`:

- Validates TOML examples against the JSON Schemas
- Runs `tofu init` + `tofu validate` (or `terraform validate`) in
  `infrastructure/terraform/`
- Lints shell scripts with `shellcheck`
- Runs `kubeconform` (or equivalent) against `infrastructure/kubernetes/`

Add `infrastructure/**` to the `docker.yml` paths filter only when the
production compose overlay or image build args change.

## 9. ADRs required

Per the convention in `docs/adr/README.md`, create infrastructure ADRs under
`infrastructure/adr/`:

- `0001-config-schema-and-loading.md` — TOML schema, precedence, Rust structs
- `0002-deployment-targets.md` — single node vs cloud vs Kubernetes, KVM requirement
- `0003-observability.md` — metric names, alert thresholds, dashboards
- `0004-cas-tier-and-backup.md` — object-store bucket, backup/restore cadence

Each ADR is a PR, reviewed, then marked `Accepted`.

## 10. Milestones and acceptance criteria

Aligned with PLAN.md:

| Milestone | infrastructure/ deliverable | Acceptance |
|-----------|-----------------------------|------------|
| M3 | config schema + defaults, `systemd/` units, `scripts/{bootstrap,deploy,smoke}.sh`, `docker/compose.prod.yaml` | `sandbox serve --config` starts (frontal-sandbox is a legacy alias); systemd units install and start; smoke.sh passes on a fresh host |
| M4 | `terraform/` for KVM host, worker systemd unit with seccomp/cgroups, Tier 2 flags | `tofu plan` clean; worker starts with `/dev/kvm` access |
| M5 | `kubernetes/`, `monitoring/`, Postgres + object-store CAS config, `backup.sh`/`restore.sh` | cluster deploys; Prometheus scrapes metrics; backup/restore round-trips a snapshot |

## 11. Conventions

- One `README.md` per subdirectory explaining purpose and how to use it.
- ADRs numbered `NNNN-short-title.md` following `docs/adr/README.md`.
- No secrets committed: `.env*` and `*.secret.toml` are git-ignored; examples
  use placeholder values.
- All shell scripts pass `shellcheck`; all TOML validates against its schema.
- TOML keys use snake_case; environment overrides use `SANDBOX_<SECTION>_<KEY>`.
