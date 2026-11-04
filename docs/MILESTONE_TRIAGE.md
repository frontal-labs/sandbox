# M3 to M5 triage

Checked 2026-11-03 against the code and `plans/PLAN.md`. Each acceptance criterion is marked
as **done** (built and tested here), **partial** (some of it is built), **not started**, or
**blocked** (needs hardware, a person, or a decision). A number is listed only if it was
measured in this pass. Otherwise it says "not measured".

## M3: server, policy, snapshots

| Item | Status | Evidence / gap |
| --- | --- | --- |
| `sandbox-policy` | partial | Crate exists and is used by the session backend. Policy depth is not reviewed here. |
| `sandbox-snapshot`: snapshot, restore, fork | partial | Snapshot, restore and fork work through the server and tests. |
| `sandbox-worker` with seccomp and cgroups | done (Linux) | Tested in the Linux container: worker isolation tests pass (456 workspace tests, 0 failed, 2026-11-03). Not run on macOS, where isolation is refused. |
| `sandbox-server` API and OpenAPI | done | OpenAPI is generated from `utoipa`; drift tests pass. |
| API keys | done | `AuthManager`, bearer keys, owner scoping. |
| `sandbox-scheduler` (single node) | done | Single-node scheduler, plus the clustered variant below. |
| Idle sleep and wake | done | ADR 0015. |
| Metering | partial | Per-key counters in memory, reset on restart. Per-command CPU is now measured (ADR 0033). Memory-time and the durable usage queue are not built. |
| **Accept:** snapshot and fork under 5 ms | **met, in memory** | Measured with `crates/sandbox-server/examples/capacity.rs`, release build, 200 iterations: snapshot p50 0.017 ms, p95 0.027 ms; fork one session p50 0.109 ms, p95 0.144 ms. The store is in memory, so disk and network cost are not included. |
| **Accept:** 1,000 concurrent sessions on one node | **met, idle, in process** | The same example creates 1,000 idle sessions in 0.07 s, at about 6.3 KiB of resident memory each. This is an in-process run of the isolate tier, not a load test on a worker. The `benchmarks/` load scenarios are scaffolds and were not used. |
| **Accept:** sleeping sessions use no memory | **not met** | The same example puts the 1,000 sessions to sleep. Resident memory then sits about 7.3 KiB per session above the idle baseline, not below it. Sleep stores its checkpoint in the in-memory store, so the memory is not released. The disk store would move it out of process, which is not measured. |
| **Accept:** escape test suite passes | partial | Isolation tests pass on Linux. The plan's list (host file reads, metadata IP, fork bombs, zip bombs, symlink races) is not fully covered. |
| Quotas, per key (measured limits) | done | 64 sessions and 32 snapshots per key, 4,096 sessions per server. The example works around them with a key per 30 sessions. |
| **Accept:** TypeScript SDK generated | partial | The TypeScript SDK is hand-written with zod schemas. It is not generated from OpenAPI, so this criterion is not met as written. |

## M4: Tier 2 (VM)

| Item | Status | Evidence / gap |
| --- | --- | --- |
| `sandbox-vmm` | partial | `VmmDriver` shells out to Firecracker. A warm-pool module exists. Nothing has booted through it. |
| `sandbox-guest` | partial | Protocol served over Unix socket and TCP. `Sync` is a placeholder that returns the nil root. |
| Guest protocol under a real kernel | done (emulated) | `docker/run-emulated-vm.sh` boots the agent under QEMU TCG. This is not Firecracker. |
| Base and browser images | not started | `images/` does not exist. |
| `export_ext4` | not started | No code. |
| `sandbox-promote` | not started | The plan says promotion logic is not built. |
| VM warm pool | done (emulated) | `VmPool` boots guests ahead of promotions, and the server takes from it. Measured under QEMU TCG: a warm handout takes under 1 ms, against a cold boot of 19.1 s. |
| Firecracker snapshots | blocked | Needs KVM. This host and the Docker VM have none. |
| **Accept:** ffmpeg and headless Chromium via auto-promotion | not started | Depends on promotion and images. |
| **Accept:** promotion under 400 ms | measured, emulated | Promoting a 64 KiB file takes 128 ms under QEMU TCG (`crates/sandbox-promote/tests/timings.rs`). That size is not the ffmpeg-scale promotion the criterion means, and TCG is not the target hardware. |
| **Accept:** VM resume under 150 ms | not measured | Resume from a snapshot is not built. The 19.1 s cold boot under TCG is the only boot time measured. |
| **Accept:** guest protocol fuzzed 24 h | partial | `guest_protocol` fuzz target exists. A 90 s run covered 1.5 M inputs with no crash. Not 24 h. |

## M5: production hardening

| Item | Status | Evidence / gap |
| --- | --- | --- |
| Clustered scheduler with Postgres | partial | `ClusterScheduler` and a Postgres store exist. Not tested against a live Postgres here. |
| Object-store CAS | not started | Only the disk and tiered stores exist. |
| Garbage collection | done | Mark and sweep, ADR 0013. |
| Multi-tenant quotas | partial | Per-session limits are enforced (ADR 0014). Tenant-level quotas are not built. |
| Billing export | not started | Usage events to a durable queue are not built. |
| Dashboards and runbooks | not started | No runbooks in the repository. |
| Metrics and tracing | not started | No Prometheus or OpenTelemetry code. |
| Chaos suite | not started | No code. |
| Documented SLOs | partial | SLO targets are written in the docs. They are not measured. |
| **Accept:** 10,000 idle sessions per host | not measured | |
| **Accept:** review findings resolved | blocked | Needs the external review. |
| External security review | blocked | Needs a person. |

## Recommended order for what can be done here

1. **Measure the M3 accept criteria.** Snapshot and fork latency, and the 1,000-session load run,
   use existing harnesses. This is the cheapest way to find out where we actually are.
2. **Metrics and tracing** (Prometheus endpoint, OpenTelemetry spans). Needed before any load
   or chaos result means much, and both are standard crates.
3. **Postgres tests** against a Postgres container, to move the clustered scheduler from partial
   to done.
4. **Object-store CAS** against an S3-compatible container (MinIO) in Docker.
5. **Chaos suite** for the cluster paths, once 3 and 4 are tested.
6. **Runbooks and SLO measurement**, after the metrics exist.

Blocked: Tier 2 Firecracker and its acceptance criteria (KVM), and the external review.
Not started and needing a decision first: billing export, tenant quotas, and the Tier 2
images, which depend on promotion.
