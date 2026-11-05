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
| **Accept:** sleeping sessions use no memory | **sessions: met on a durable server. Overall: not met, because each sleep's snapshot record stays in memory.** | A durable server parks each sleeping session as one file in `sessions/`, and its table entry leaves memory. Per-key and per-tenant counts, and the checkpoint budget, still include parked sessions. Measured in a fresh process (`examples/parked_restart`): an empty data directory opens at 4,416 KiB. The same directory with 1,000 parked sessions opens at 12,816 KiB. A control with 1,000 snapshot records and no sessions opens at 12,668 KiB. So a parked session adds about 0.15 KiB, and the rest is the snapshot record that each sleep creates (about 8.2 KiB). Durable snapshots take about 20 ms, against 0.017 ms in memory. | Only the durable store parks sessions. A memory-only server keeps sleeping sessions in the table, as before, and the clustered server does too, because its data directory is shared. Moving snapshot records to disk is the next step. It is a separate table, with its own per-key limit and its own listing. |
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
| Firecracker snapshots | not built | Firecracker is not used: Tier 2 runs on QEMU, with KVM on the hosted runner (`vm-kvm` workflow, passing on `aed112c`) and software emulation elsewhere. Memory snapshots are not working (see VM resume). |
| **Accept:** ffmpeg and headless Chromium via auto-promotion | not started | Depends on promotion and images. |
| **Accept:** promotion under 400 ms | measured, emulated | Promoting a 64 KiB file takes 128 ms under QEMU TCG (`crates/sandbox-promote/tests/timings.rs`). That size is not the ffmpeg-scale promotion the criterion means, and TCG is not the target hardware. |
| **Accept:** VM resume under 150 ms | not met | Resume from a memory snapshot is not working. QEMU's migration stream reaches its full size, but the host never sees the end of the stream, so the snapshot hangs (`crates/sandbox-vmm` history). Cold boot under KVM on a hosted runner is 7.8 s, and under TCG 19–31 s. |
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

Blocked: the external review. The KVM runner now runs the Tier 2 suite; the VM resume criterion is not met.
Not started and needing a decision first: billing export, tenant quotas, and the Tier 2
images, which depend on promotion.
