# 0015: Idle sleep and metering

## Status
Accepted

## Date
2026-10-19

## Context
PLAN.md §4.14 calls for an idle policy: after a while without activity, a session is snapshotted and its memory freed, and the next request wakes it. Until now sleep was explicit only (ADR 0006), and a request to a sleeping session returned `409`. The plan also asks for metering, and the API reference lists `GET /v1/usage`, which had no route.

## Decision
- **Activity.** Every request that looks up a session stamps it with the current time (`last_active`). Wake stamps it too.
- **Idle sleep.** `sandbox_server::idle::sweep` puts to sleep every live session whose last activity is older than the threshold. A session whose gate is held (a command or file operation is running) is skipped, not waited for, and a session is checked again under its gate before it sleeps, so a request that arrived during the scan is not put to sleep. `sandbox serve --idle-sleep-secs` runs the sweep, default 300 seconds, 0 turns it off. (`frontal-sandbox` is a legacy alias.) The sweep runs at least every second and at most every 30 seconds.
- **Wake on request.** A request to a sleeping session (exec, file operations, snapshot) wakes it from its checkpoint and then proceeds. This replaces the `409` that ADR 0006 specified. Explicit `POST /v1/sessions/{id}/wake` still returns `409` for a session that is already running, and `POST /sleep` still returns `409` for one that is asleep. Get and destroy never wake.
- **Metering.** Each key has counters since the server started: sessions created, slept, and woken; commands run; command wall time in milliseconds; and bytes written through the filesystem API. `GET /v1/usage` returns the caller's counters.

## Consequences
### Positive
- A session nobody uses holds no memory after the idle threshold, as PLAN §4.14 requires.
- A client does not need to know whether a session sleeps. The next request just works, at the cost of the wake.

### Negative
- A wake costs a restore from the checkpoint, so the first request after an idle period is slower.
- The counters are in memory and reset on restart. They are not billing records. Usage events to a durable queue (PLAN §4.14) and memory-time and CPU-time metering are not implemented. CPU time is not measured, and memory is not limited, so there is no memory-time to meter yet.
- A sweep does not stop a command, so a command that runs past the threshold keeps its session awake until it finishes.

## Related
- [ADR 0006](0006-sleep-and-wake.md): sleep and wake. Its statement that requests to a sleeping session return `409` is superseded by this ADR.
- [ADR 0013](0013-blob-store-garbage-collection.md): the collector also waits on session gates, so a long command delays it too.
