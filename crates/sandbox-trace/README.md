# sandbox-trace

Event log and replay for the sandbox system.

## Status

Early / partial. `TraceLog` is an in-memory, async event queue. `ReplayEngine::replay` is a stub that returns `completed: true` with no divergences and does not re-execute anything. Not yet implemented: JSONL persistence, sequence numbers, `diff`, Parquet or ClickHouse sinks, OpenTelemetry, and Prometheus metrics. No tests exist.

## Overview

Per-session event log built on the `Event` enum from `sandbox-types`. Source: `crates/sandbox-trace/src/`.

## Public API (from `src/lib.rs`)

- `TraceLog` (`log.rs`): `new()`, `append(event).await`, `events().await -> Vec<Event>`. Implements `Default`.
- `replay` module (`replay.rs`): `ReplayConfig` (`deterministic`, `virtual_clock`; default both `true`), `ReplayEngine` (`new`, `replay`), `ReplayResult` (`divergences`, `completed`). Stub.

## Invariants and security assumptions

- Events are appended in call order to a shared queue; this is not yet a monotonic sequence number scheme.
- Trace events may contain command output (`StdoutChunk`, `StderrChunk`) and file data. Persisted traces must be treated as sensitive, and secret values must not be recorded.
- Replay determinism is a goal, not a current guarantee.

## Example

```rust
use sandbox_trace::TraceLog;
use sandbox_types::{Event, SessionId};
use std::time::SystemTime;

# async fn demo() {
let log = TraceLog::new();
log.append(Event::SessionDestroyed {
    session_id: SessionId::new(),
    timestamp: SystemTime::now(),
})
.await;
let events = log.events().await;
# }
```

See `PLAN.md` section 4.12 for the target spec.
