# 0002: Deployment targets

## Status
Accepted

## Date
2026-10-17

## Context
Frontal Sandbox must run from a single laptop to a multi-node cluster. PLAN.md
defines M3 (single node), M4 (cloud host with Tier 2), M5 (clustered). We need
to decide which targets this infrastructure directory supports first and how
they relate.

## Decision
Three targets, implemented in order:

1. Single node (M3): static `sandbox` binary + systemd units + container image
   (`docker/` + `infrastructure/docker/compose.prod.yaml`). Tier 1 only unless
   the host has `/dev/kvm`.
2. Cloud host (M4): OpenTofu/Terraform provisions a KVM-capable instance
   (`infrastructure/terraform/`), then `deploy.sh` installs systemd units.
3. Clustered (M5): Kubernetes manifests (`infrastructure/kubernetes/`) with
   Postgres metadata, object-store CAS, Prometheus/Grafana.

All targets share the same config schema (ADR 0001).

## Consequences
### Positive
- Clear milestone alignment; operators pick the target matching their scale.
- Single-node path reuses the existing `docker/Dockerfile` and `compose.yaml`.

### Negative
- Kubernetes manifests are aspirational until `sandbox-scheduler` and
  `sandbox-worker` expose gRPC and the config surface (M3/M4).
- Terraform targets AWS by default; other providers require editing `main.tf`.

## Alternatives Considered
- Nomad / ECS / Cloud Run: rejected; the project targets operator-provisioned
  hosts with KVM access for Tier 2, which serverless platforms do not provide.

## References
- PLAN.md sections 4, 5, 6, 10
- docs/DEPLOYMENT.md
