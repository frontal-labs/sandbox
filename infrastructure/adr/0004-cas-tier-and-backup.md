# 0004: CAS tier and backup

## Status
Accepted

## Date
2026-10-17

## Context
`sandbox-cas` has `MemStore`, `DiskStore`, `TieredStore`, and a planned
`ObjectStore` (S3-compatible). M5 moves metadata to Postgres and CAS to an
object store with a local cache. We need the infrastructure to provision the
bucket and to back up/restore state.

## Decision
- `infrastructure/terraform/storage.tf` provisions an S3 bucket (versioning +
  lifecycle) for the CAS tier; `scheduler.cas.backend` selects `disk`/`tiered`/
  `object`.
- `infrastructure/scripts/backup.sh` tars `/var/lib/sandbox` and uploads to a
  local dir or `s3://`; `restore.sh` reverses it.
- Metadata (Postgres) dump/restore is operator-managed until M5 clusters it.

## Consequences
### Positive
- CAS objects are durable beyond a single host's disk.
- Backup/restore round-trips keys, snapshots, and sleeping sessions.

### Negative
- `state.json` is rewritten in full per change (ADR 0008); backup captures a
  point-in-time snapshot, fine for M3/M4, replace with log-structured store in M5.
- Postgres metadata backup is not automated here.

## Alternatives Considered
- Restic / borg: rejected; a simple tar + object-store copy is sufficient for
  M3/M4 and avoids a new dependency.

## References
- plans/PLAN.md sections 4.2, 4.14
- docs/adr/0008-durable-server-state.md
- `infrastructure/scripts/`
