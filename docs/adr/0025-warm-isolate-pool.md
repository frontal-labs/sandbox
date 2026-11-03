# 0025: Warm isolate pool and the cold-start measurements

## Status
Accepted.

## Date
2026-10-20

## Context
PLAN §6 sets the cold start of a Tier 1 session at under 5 ms from a warm pool, and PLAN §4.14 describes pools of pre-warmed isolates. The measurements came first, so the pool would go where the cost is.

Measured in a Linux release build (`frontal-sandbox bench`, 200 iterations after warm-up, the container on Apple silicon):

| Case | p50 | p99 |
| --- | --- | --- |
| `session_create` (a new Tier 1 session) | 833 ns | 1.2 µs |
| `node_script` before the pool (a fresh V8 isolate per run) | 5.10 ms | 6.94 ms |

A session costs well under the target without any pool, because it creates no V8 isolate. A `node` run built a new isolate every time, and that was the cost.

## Decision
- **A small warm pool for `node`.** The runtime keeps `WARM_TARGET` (2) isolates built ahead of need. Each sits on its own thread, because V8 isolates are not `Send`. A `node` run takes one, and the pool starts a replacement in the background.
- **Only the default heap limit uses the pool.** Another limit builds its own isolate, as before.
- **A run never waits for the pool.** If no isolate is ready, the run builds one. The cold path is still there and is tested.
- **Isolates stay single-use.** A warm isolate runs one script and is dropped, so no state carries from one run to the next. A test checks this.

## Measured result
With the pool, `node_script` p50 is 0.41 ms, down from 5.10 ms. The p90 is back at 4.9 ms, because the benchmark runs scripts back to back. Each takes about 0.4 ms, and a replacement takes about 5 ms to build, so the pool empties under that load. A pool of four did not change the tail, so the bigger pool was not kept. A workload with gaps between `node` runs hits the pool; a tight loop of runs does not.

## Consequences
### Positive
- The median `node` run is about 12 times faster.
- The timeout, output cap, heap limit, and isolation between runs are tested on the warm path.

### Negative
- Idle isolates hold memory: two V8 isolates, which is the price of the pool.
- Under sustained back-to-back load the tail is the cold cost.

## Alternatives Considered
- **V8 snapshots.** Not built yet. A snapshot would make the cold path faster too, and it is the next thing to try, measured the same way.
- **A larger pool.** Measured, and it did not help this load. Replacement time, not pool size, sets the tail.

## References
- [PLAN.md §4.14, §6](../../PLAN.md).
- `crates/sandbox-runtime/src/runtime.rs` (`WARM_TARGET`, `run_warm`), `crates/sandbox-runtime/tests/warm_isolates.rs`, `cli/src/bench.rs` (`session_create`, `node_script`).
