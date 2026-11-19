# M5 — Clustered chaos suite

**Verdict: PASS (4/4) — new clustered chaos suite written and passing against live
Postgres; plus a real clustered compile bug found and documented.**

## 1. New test suite: `crates/sandbox-server/tests/chaos_clustered.rs`

Worktree `t3-25a2e45a`, commit `3c94f20`. Four tests, all passing against a live
`postgres:16` container (`chaos-pg` @ 127.0.0.1:55440):

```
test a_save_is_atomic_and_a_crash_leaves_the_previous_document ... ok
test a_damaged_body_refuses_to_open ... ok
test a_wrong_version_refuses_to_open ... ok
test a_stale_node_cannot_overwrite_a_newer_revision ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; finished in 1.75s
```

What each covers:
- **Atomic save**: `PgStore::save` (conditional `INSERT ... ON CONFLICT DO NOTHING` for
  revision 0, `UPDATE ... WHERE id=1 AND revision=$2` otherwise) is a single statement —
  a "crash" mid-save (simulated by never writing) leaves the previous document intact;
  a failed save never corrupts the row.
- **Damaged body**: a `body` that violates the document schema but carries the right
  version is refused on open (surfaces as `ErrorCode::Internal`, documented).
- **Wrong version**: opening with a revision that doesn't match the stored one is
  refused.
- **Stale node**: a node holding an older revision cannot overwrite a newer one —
  the conditional UPDATE matches 0 rows and the caller sees the conflict.

Gated behind `--features clustered` and `SANDBOX_TEST_DATABASE_URL` (fails fast with a
clear message when unset), so it stays out of the default `cargo test` run.

## 2. Real bug found: clustered build broken by `BillingLog::open` (commit `a7b52e5`)

- `crates/sandbox-server/src/billing.rs`: `BillingLog::open` (the file-backed billing
  log) is `cfg(not(clustered))`, but `open_durable_with_options` (the clustered durable
  open path) called it **unconditionally** — so the crate did not compile with
  `--features clustered` at all.
- Introduced by `9a66fc0` (#108). The experiment commit gates the file-backed
  `open_durable` under `not(clustered)` and documents this. Marked **NOT FOR MERGE**
  as-is: the right fix is a shared-durable billing log for clustered mode (or a
  Postgres-backed one), which is a design decision, not a one-liner.

## Gap / notes
- The chaos-suite acceptance line ("Chaos suite passes") is now demonstrable: the suite
  exists and passes.
- What is still missing for a full M5 clustered story (out of scope for this item):
  multi-node scheduler, object-store (S3) CAS, and the proper clustered billing log.
  The `a7b52e5` gate is a stopgap so the clustered feature compiles at all.
- Re-run command (after `CREATE DATABASE sandbox` in the test Postgres):
  `SANDBOX_TEST_DATABASE_URL=postgresql://postgres:***@127.0.0.1:55440/sandbox \
   cargo test -p sandbox-server --features clustered --test chaos_clustered`
