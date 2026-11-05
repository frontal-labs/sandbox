# 0034: Parked sleeping sessions

## Status
Accepted

## Date
2026-11-05

## Context
A sleeping session kept its table entry in memory: the checkpoint, the trace, and the
bookkeeping. The plan's acceptance criterion is that sleeping sessions use no memory. The
state log also loaded every sleeping session into memory when the server started. The
durable state is one base file and a log (ADR 0016), so no single file can hold one session
without rewriting the whole state on every sleep.

## Decision
On a durable server, sleeping a session writes its record to `sessions/<id>.json` in the
data directory, and removes its entry from the table. The record is the same `SavedSession`
the state log uses. The state log holds awake sessions only.

- **Lookup.** A request for an ID that is not in the table reads its parked record. The
  owner must match, and the session is reported as sleeping.
- **Wake.** The session is restored into the table, the state log is saved with it awake,
  and only then is its parked record removed.
- **Sleep.** The record is written first, and only then does the entry leave the table.
- **Restart.** A sleeping session in the state log is written to `sessions/` if no record is
  there. A parked record for a session the state log has awake is stale, and is removed.
- **Limits and budget.** Parked sessions are counted per key and per tenant, and their pinned
  checkpoint bytes count against the key's budget. The counts are per key and tenant, not per
  session, so they cost no memory per sleeping session.
- **Garbage collection.** A parked record's roots and spilled outputs are marked live.

The ordering is what makes a crash safe. Each step leaves the session either awake or parked,
never lost. A parked record left behind after an interrupted wake is removed at the next
start, because the state log already holds the session awake.

Memory-only servers keep sleeping sessions in the table, as before. The clustered server also
keeps them there, because its data directory is shared between nodes and the counts are per
node.

## Consequences
- Measured in a fresh process (`examples/parked_restart`): 1,000 parked sessions add about 0.15 KiB
  each to a restarted server. A control with 1,000 snapshot records and no sessions adds about 8.2 KiB
  each. See `docs/MILESTONE_TRIAGE.md`.
- Each sleep still writes a snapshot record, which the server keeps in memory (about 8 KiB).
  Moving snapshot records to disk is the next step; until then the criterion is met for sessions
  but not for the sleep as a whole.
- Every parked session costs a file. Interrupted writes can leave a `.tmp` file in
  `sessions/`; startup ignores those.
- Waking, reading a trace, or reading checkpoints of a sleeping session reads its file. These
  are slower than a table lookup, and the cost is per request, not per server.
