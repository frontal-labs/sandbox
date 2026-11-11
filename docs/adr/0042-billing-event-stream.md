# 0042: Durable billing event stream in Postgres

## Status
Accepted

## Date
2026-11-11

## Context
PLAN §4.14 asks for usage events to a durable queue. Per-session metering (ADR 0041) keeps the
meters in memory, and the usage export is a snapshot. Nothing recorded when a session started or
stopped, and a restart lost the meters. A billing consumer needs a record that survives restarts
and that it can resume from.

## Decision
- A `billing_events` table in the clustered Postgres database holds the stream. The `seq`
  sequence is the position: a consumer saves the last `seq` it handled and resumes after it.
  Postgres was chosen over the object store because positions must be ordered and must be
  readable by any node, and the clustered state already lives there (ADR 0023).
- Three kinds of event: `session.started` (admission of a new session), `usage.reading` (after
  each command that finished, with the session's cumulative meter), and `session.stopped` (on
  delete, with the final meter).
- An event is written before the request that caused it is answered. If the write fails, the
  request fails: a session is not admitted without its start event, and a command's reading that
  cannot be written is reported as an error. Acknowledged events are therefore durable.
- `GET /v1/billing/events?after=SEQ&limit=N` returns the caller's events after `SEQ`, oldest first,
  one JSON object per line. Each event carries its `seq`. Only the caller's own events are returned.
- On startup, the meters are rebuilt from the stream: each session's latest reading gives its
  cumulative meter, and a session with a `session.stopped` event comes back stopped. A session
  that never ran a command comes back with a zero meter.
- Only a clustered server has the stream. On other builds the same types exist with no values, so
  the endpoint answers `404` and nothing is appended.

## Consequences
### Positive
- No acknowledged event is lost across a restart; the clustered test checks this.
- A consumer resumes from any saved position, and gets exactly the events after it.
- Meters survive a restart, so the usage export keeps the totals.

### Negative
- A reading is one database write per command. On a busy server this is the cost to watch; the
  stream holds every command, not a sample.
- The stream grows without bound. Retention and compaction are not implemented.
- A failed reading write turns a finished command into an error for the caller, who cannot tell
  from the reply whether the command ran. The command's effects remain in the filesystem.

## Alternatives Considered
- The object store, as one object per event or per batch. Not chosen: positions would need a
  separate index, and each append would be a remote write without ordering.
- Batching readings in memory and flushing periodically. Not chosen: a restart would lose the
  unflushed batch, which the requirement forbids.

## References
- PLAN §4.14, §4.8. ADR 0041 (metering), ADR 0023 (Postgres state), ADR 0021 (usage export).
