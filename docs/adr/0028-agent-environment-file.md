# 0028: Agent environment file

## Status
Proposed

## Date
2026-10-25

## Context
An agent running in a session has no way to learn what the session is: its ID, its tier, its user, which commands exist, what the network policy allows, or what limits apply. IMAGE §4.5 asks for a machine-readable file at `/etc/agent/env.json`, written by the host, with the same layout in both tiers so that promotion is invisible to the agent. IMAGE §9 (I0) makes it an acceptance criterion: every Tier 1 session has a valid file.

Three things about the current code shape the design:

- A session's ID is set when its backend is created. `restore_checkpoint_limited` gave a restored backend a fresh random ID. After a wake or a restart, the backend's ID differed from the ID the server keeps for the session. A file that carries the session ID would have been wrong after any restore.
- Quotas count every file in the session's tree (ADR 0014). Any file the host writes counts too.
- The REST API and the Tier 1 shell share one filesystem. The file is visible to the agent through the filesystem routes and through `cat`.

## Decision
- **Schema v1** (`sandbox_types::agent_env`): `schema`, `session`, `tier`, `user`, `image`, `packages`, `network`, `limits`, `gui`, `commands`, with the field names in IMAGE §4.5. The Rust types are the source of truth. Tests pin the key names.
- **Values Tier 1 reports:**
  - `session`: the session's ID, written as the bare ULID the REST API uses. IMAGE's example shows a `ses_` prefix, which the codebase does not use. This is a deviation from IMAGE, and it keeps one ID format.
  - `tier`: `"isolate"`.
  - `user`: `"agent"`. Tier 1 has no users. The name is reported so that the agent sees one contract in both tiers. IMAGE §4.5 asks for this.
  - `image`: `base` is `null` and `layers` is empty. Tier 1 has no layers before I3 (ADR 0026).
  - `packages`: empty.
  - `network`: the session's policy. Tier 1 accepts only the default, so this is `deny` with no allow rules.
  - `limits.wall_timeout_s`: the command cap the tier enforces (300 s).
  - `limits.memory_mib`, `limits.cpu_ms_budget`: `null`. Neither is enforced in Tier 1 (ADR 0014). IMAGE's example shows numbers, which would describe limits that do not apply.
  - `gui`: unavailable. `commands`: `["sandbox"]`.
- **When it is written.** The backend writes the file when it starts, which covers creation, a restore under the same ID, and a restore under a new ID (a fork). The file is written before the session's quotas apply. A session with a small quota still has its file.
- **Identity on restore.** A restore takes the session's own ID: `restore_session_limited(id, checkpoint, limits)`. A wake and a restart use it, so the file always names the session that owns it. The other restore, `restore_checkpoint_limited`, still gives a new ID. The file is rewritten on every start. Writing the same contents again gives the same root, which a test checks, so a restore does not change the state a snapshot records.
- **The command.** `sandbox info [--json]` prints the file. `--json` is accepted because the file is JSON already. The builtin is in the shell and reads the VFS, so it needs no host call and works in scripts and in traces.
- **Secrets.** Environment values and secret names are not written. `network.allow` lists host patterns only.

## Decisions for review
- **The file counts against quotas.** Every file counts, including the host's. A session's `max_total_size` and `max_inodes` leave room for three inodes (`/etc`, `/etc/agent`, and the file) and the file's bytes. A total-size quota smaller than the file refuses every write. The session still starts. The alternative is to exempt `/etc/agent` from the VFS accounting. That needs a change to the VFS usage walk and a rule for when the agent writes there, so it is not part of this change.
- **Restores take the default network policy.** A checkpoint does not record a network policy, and Tier 1 accepts only the default. If Tier 1 ever accepts another policy, the checkpoint must record it, and a restore must read it back.

## Consequences
### Positive
- An agent can read its own session identity, tier, limits, and policy without a special API.
- The restore path keeps the session's ID. This fixes a mismatch that was not visible before, because nothing reported the backend's ID after a restore.
- The file has the same shape in both tiers, which is IMAGE's promotion requirement.

### Negative
- Every session carries an extra three inodes and about 600 bytes against its quotas.
- The file is regenerated on every restore, which makes restores write to the overlay. The root does not change, but the write happens.

### Risks
- A later schema change must bump `schema` and keep old readers working, or the agent gets a file it cannot parse. The version field exists for that.

## Alternatives Considered
- **Serve the file from the REST API only.** Rejected. IMAGE asks for it in the session, where the agent runs.
- **Store the file outside the VFS, in a virtual mount.** Rejected for v1. It would need the same read path changes as the trace view, and the file is ordinary session state that snapshots and forks should carry.
- **Write `env.json` only at creation.** Rejected. A fork and a restore under a new ID would carry the parent's ID.

## References
- plans/IMAGE.md §4.5, §9 (I0), §12 (question 5, the agent fork quota, which the fork design uses)
- [ADR 0006](0006-sleep-and-wake.md), [ADR 0010](0010-durable-live-sessions.md): sleep, wake, and restart keep the session
- [ADR 0014](0014-session-limits.md): quotas and the limits the tier enforces
- [ADR 0026](0026-package-layers-and-lockfile.md): the image fields are empty until layers exist
