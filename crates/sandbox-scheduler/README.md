# Sandbox Scheduler

Scheduler for the sandbox system: session bookkeeping, worker placement, and a warm-isolate pool stub.

**Status: partial, in-memory only.** `Scheduler` tracks sessions and workers in memory. Placement is least-loaded in memory, and it is not yet used to bind a session to a worker. The pool is a `Vec` with no pre-warming. There is no metadata store, no idle sleep, no metering, and no admission control. No tests exist yet.

## Public API

Re-exported from `src/lib.rs`: `Scheduler`. The modules `placement` and `pools` are also public.

- `Scheduler` (`src/scheduler.rs`)
  - `new()`: no arguments. There is no config or metadata store parameter.
  - `async create_session(config: SessionConfig) -> Result<SessionId, SandboxError>`: records a `SessionState` with `Tier::Isolate`. It tries a pooled isolate and a placement, but the placement result does not change the returned session. Treat placement as not yet functional.
  - `async destroy_session(id)`, `async get_session(id) -> Option<SessionState>`
  - `async register_worker(WorkerInfo)`, `async unregister_worker(&str)`
  - `async pool_stats() -> PoolStats`
- `placement` (`src/placement.rs`): `PlacementStrategy` (least-loaded by in-memory count), `WorkerInfo`, `PlacementDecision`.
- `pools` (`src/pools.rs`): `IsolatePool`, `PoolConfig` (default min 1, max 10, idle timeout 300 s), `PooledIsolate`, `PoolStats`. `acquire` pops a `Vec` entry, so the pool is empty until something releases into it.

## Planned design

- Placement by tenant affinity, then free memory.
- `MetaStore` trait with a single-node `redb` backend and a clustered Postgres backend behind the `clustered` feature. The `redb` and `sqlx` dependencies are declared, but no `MetaStore` exists yet.
- Warm pools sized by demand, idle sleep (snapshot then free worker), wake on request.
- Metering (CPU-ms, memory GB-s, egress bytes, VM seconds) emitted to a durable queue.
- Admission control: `429` with `Retry-After` when over quota or capacity.

## Invariants and security assumptions

- Session state lives only in process memory today. A restart loses it.
- No tenant isolation is enforced by the scheduler yet. `TenantId` is stored on the session but not used for placement or quota.

## Example

```rust
use sandbox_scheduler::Scheduler;
use sandbox_types::{SessionConfig, TenantId};

async fn start(tenant: TenantId) -> Result<(), Box<dyn std::error::Error>> {
    let scheduler = Scheduler::new();
    let id = scheduler.create_session(SessionConfig::new(tenant)).await?;
    assert!(scheduler.get_session(id).await.is_some());
    Ok(())
}
```

Not compiled in this pass. Checked against `src/scheduler.rs` and `sandbox-types/src/session.rs`.

## Spec

See the project specification section 4.14 and milestone M3. The `redb`-based `RedbMetaStore` and `SchedulerConfig` shown in earlier drafts of this README do not exist.
