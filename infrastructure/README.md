# infrastructure/

Deployment, configuration, provisioning, and operations for Frontal Sandbox.

Nothing in this directory is runnable until `crates/sandbox-server`,
`crates/sandbox-scheduler`, and `crates/sandbox-worker` expose the config and
CLI surface described in [PLAN.md](PLAN.md).

## Contents

| Path | Purpose |
|------|---------|
| `PLAN.md` | Comprehensive implementation and structure plan |
| `config/` | TOML config schema, defaults, and per-environment examples |
| `systemd/` | Single-node systemd units (server, scheduler, per-tenant worker) |
| `terraform/` | OpenTofu/Terraform for KVM-capable cloud hosts |
| `docker/` | Production compose overlay for the local stack |
| `kubernetes/` | Clustered manifests (M5) |
| `monitoring/` | Prometheus alerts, recording rules, Grafana dashboards |
| `scripts/` | bootstrap, deploy, upgrade, backup, restore, smoke |
| `ci/` | CI glue for infrastructure changes |
| `adr/` | Infrastructure architecture decision records |

## Quick start (single node)

```bash
# From the repository root, after building the `sandbox` binary:
sudo infrastructure/scripts/bootstrap.sh
sudo infrastructure/scripts/deploy.sh \
    --binary target/release/sandbox \
    --config infrastructure/config/examples/dev.toml
infrastructure/scripts/smoke.sh http://127.0.0.1:8080
```

## Conventions

- TOML config; `SANDBOX_<SECTION>_<KEY>` env overrides take precedence.
- No secrets committed. Examples use placeholder values.
- Shell scripts pass `shellcheck`; TOML validates against its JSON Schema.
- ADRs live in `adr/`, numbered `NNNN-short-title.md`, following
  `docs/adr/README.md`.

## Milestone alignment

- M3: config schema + defaults, systemd units, deploy/smoke scripts, compose overlay
- M4: Terraform for KVM hosts, worker isolation flags
- M5: Kubernetes manifests, Prometheus/Grafana, backup/restore

See [PLAN.md](PLAN.md) section 10 for acceptance criteria.

## License

Apache-2.0
