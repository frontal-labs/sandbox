# deploy/

Concrete, installable deployment artifacts for Frontal Sandbox.

These are the artifacts an operator installs on a host: the production
container image, systemd unit files, the deploy/bootstrap/upgrade/smoke
scripts, and an example config. They complement `infrastructure/`, which holds
the deployment plan, config schemas, cloud provisioning (Terraform), cluster
manifests (Kubernetes), monitoring, CI glue, and ADRs.

## Contents

| Path | Purpose |
|------|---------|
| `Dockerfile` | Production-hardened container image (TLS-ready, non-root, tini) |
| `compose.prod.yaml` | Production compose stack extending `docker/compose.yaml` |
| `config.example.toml` | Example operator config (copy to `/etc/sandbox/config.toml`) |
| `.env.example` | Compose environment overrides |
| `systemd/` | Installable systemd unit files |
| `scripts/` | bootstrap, deploy, upgrade, smoke, backup, restore |

## Quick start (single node)

```bash
# As root on a KVM-capable Linux host:
sudo ./scripts/bootstrap.sh
sudo ./scripts/deploy.sh \
    --binary /path/to/target/release/sandbox \
    --config config.example.toml
./scripts/smoke.sh http://127.0.0.1:8080
```

## Milestone alignment

- M3: Dockerfile, compose.prod.yaml, systemd units, deploy/smoke scripts, config example
- M4: deploy.sh Tier 2 (KVM/Firecracker) flags, bootstrap.sh kvm enablement
- M5: backup.sh/restore.sh for object-store CAS + Postgres metadata

See `infrastructure/PLAN.md` for the full plan and acceptance criteria.
