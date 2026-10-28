# 0023: Clustered durable state on Postgres

## Status
Accepted. It implements the Postgres store of PLAN §4.8 and M5 for the metadata. The blobs and the clustered scheduler are separate, and they are still open.

## Date
2026-10-20

## Context
The file store (ADR 0016) keeps the state of one node in a data directory. Several nodes that serve the same sessions need one shared state, and an update from one node must not silently overwrite another's. PLAN §4.14 names Postgres via `sqlx` for clustered mode.

## Decision
- **The whole saved state is one row.** `sandbox_state` holds the `Saved` document as `JSONB`, with a `revision` column. The row is keyed by `id = 1`.
- **Each save writes the whole document, conditionally.** The update is `WHERE revision = <the revision this node last read or wrote>`. If no row was updated, another node wrote first. The save fails with `InvalidState`, writes nothing, and the caller reloads and retries. Two nodes cannot overwrite each other.
- **Version is checked before the rest of the document is read.** A state of another version is refused as such, not reported as a malformed document.
- **Blobs stay in the blob directory.** Every node must reach the same `--data-dir`, such as a shared volume. The database holds the metadata only.
- **It is a build choice.** `--features clustered` builds the Postgres store, and `serve --database-url` (or `SANDBOX_DATABASE_URL`) selects it. A build without the feature refuses the option with a clear message.

## Consequences
### Positive
- The write is simple and checkable. There is no log to replay and no merge to get wrong, and the conflict is detected by the database.
- The tests run against Postgres 18: a restart reads the saved state, a stale node's save is refused and writes nothing, a reloaded node can save again, and a state of another version is refused.

### Negative
- Each save writes the whole state. For a large state this costs more than the file store's appends. That is the trade for simplicity, and the file store stays the default.
- The blob directory is still a single-node assumption. A multi-node deployment needs a shared filesystem until blobs move to object storage.
- A save that loses a race returns `InvalidState`, and the caller must retry. The server does not retry on its own yet.

## Alternatives Considered
- **Incremental rows per entity.** Rejected for now. It needs per-entity versioning and a delete protocol, and the full-document write is correct first. It is the next step if the state grows.
- **Advisory locks instead of a revision.** Rejected. A lock held across a request ties the server to one connection, and it fails badly when a node dies holding the lock.
- **One table per entity type.** Rejected. It spreads one consistency rule over several tables.

## Verification
- `SANDBOX_TEST_DATABASE_URL=postgres://… cargo test -p sandbox-server --features clustered --test clustered` against Postgres 18.6: two tests pass.
- The tests fail without the variable, so a run without a database is never reported as a pass. They reset the `sandbox_state` table, so the database must be used only for tests.

## References
- [ADR 0016](0016-log-structured-durable-store.md): the file store.
- [PLAN.md §4.8, §4.14](../../PLAN.md).
- `crates/sandbox-server/src/pg_store.rs`, `crates/sandbox-server/src/persist.rs` (`Persistence`), `crates/sandbox-server/tests/clustered.rs`.
