# 0049: Snapshot templates

## Status
Accepted.

## Date
2026-11-12

## Context
Snapshots (ADR 0004) capture a committed filesystem so a session can start from a
pre-built state. But a snapshot has no human name and no way to say "this is the
standard environment I want to start from". A caller that wants a stable, reusable
starting point — say, a repo already checked out with dependencies installed — had
to copy the snapshot ID around by hand and could not reference it by a meaningful
name. We want a thin, named, owner-scoped layer over an existing snapshot so a
session can start from a pre-built filesystem and tool state by name.

## Decision
- A **template** is a distinct type over an existing snapshot: it holds a `name`
  and a `snapshot_id`, owned by the key that created it. It is not a renamed
  snapshot and does not copy the snapshot's `root`, `limits`, or `allow_vm` — those
  are read from the snapshot record each time the template is used. A template is a
  stable name on top of a snapshot record.
- Templates are **key-scoped** like snapshots (ADR 0004): only the creating key can
  list, read, or start a session from a template. Any other key gets `not_found`.
- The record and in-memory table live in the **server** (`api.rs`), mirroring
  `SnapshotRecord`/`SnapshotTable`. `sandbox-snapshot` gains no manager.
- A session starts from a template via a new `SessionConfig.initial_template`
  field, kept separate from `initial_snapshot` for type safety. Setting both is
  refused with `invalid_argument`. The template arm resolves the wrapped snapshot
  and restores it with the same `restore_limited(root, limits)` path as the
  snapshot arm.
- A template can only wrap a snapshot the caller owns; wrapping another key's
  snapshot is refused with `not_found`.
- Minimal routes: `POST /v1/templates`, `GET /v1/templates`,
  `GET /v1/templates/{id}`. There is no delete in this milestone.
- Templates persist across restarts like snapshots: `SavedTemplate` in
  `persist.rs`, stored in `state.json` (the log-structured store reuses the same
  record types, so the clustered Postgres backend needs no change).
- The proto surface adds a `TemplateId` message and an
  `optional TemplateId initial_template` field on `SessionConfig`.

## Consequences
### Positive
- A session can start from a pre-built filesystem by a stable, owner-scoped name
  instead of juggling raw snapshot IDs.
- Templates are cheap: no filesystem copy, no denormalised limits. If the snapshot
  record is the source of truth, templates always agree with it.
- Ownership and persistence follow the proven snapshot pattern, so the security
  model and the durable-state code stay small and consistent.

### Negative
- A template's behaviour can change if the underlying snapshot record changes
  between when the template was created and when it is used, because limits and
  root are read live. A template is a pointer, not a pinned copy.
- Templates cannot be deleted in this milestone, so a key can accumulate names.
- There is no server-side cap on the number of templates per key yet.

### Risks
- If a template outlives the snapshot it wraps (e.g. the snapshot is garbage
  collected), starting from the template will fail with `not_found` rather than
  degrading gracefully. This is acceptable for the milestone but should be
  revisited if templates are expected to be long-lived.

## Alternatives Considered
- **Rename a snapshot** and reuse it: rejected, because a template needs its own
  identity and owner and must not mutate the snapshot it references.
- **Denormalise `root`/`limits`/`allow_vm` into the template**: rejected, because
  it duplicates state that already has one source of truth and would drift if the
  snapshot record changed.
- **Put a `TemplateManager` in `sandbox-snapshot`**: rejected, to keep the
  template as a server concern that mirrors snapshot ownership rather than a
  snapshot-store concern.

## References
- ADR 0004 (snapshot records live in the server), ADR 0008 (durable server state),
  ADR 0016 (log-structured durable store).
