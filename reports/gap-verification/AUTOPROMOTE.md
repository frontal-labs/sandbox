# M4 — ffmpeg + headless Chromium auto-promotion, promotion < 400 ms

**Verdict: PASS — real ffmpeg and headless Chromium executions run through auto-promotion
(63 s end-to-end), and promotion of a small workspace is 9 ms vs the 400 ms target.**

## 1. `vm_auto_promote_real` (end-to-end, `#[ignore]` test)

- Worktree: `t3-0d31d18e`, test `crates/sandbox-server/tests/vm_auto_promote_real.rs`.
- Server built in release with clustered feature; guest image built with
  `docker/build-guest-image.sh` (aarch64: vmlinuz 32,956,352 B + initrd.gz 286,302,926 B)
  into `/tmp/guest-image`, booted under QEMU **HVF** (`SANDBOX_VM_ACCEL=hvf`,
  `SANDBOX_VM_ARCH=aarch64`).
- The test creates a session, runs an exec that requests a VM-tier workload (ffmpeg
  transcode + headless Chromium page render) over `POST /v1/sessions/{id}/exec` via
  HTTP; the server auto-promotes the isolate session to a microVM and the workload
  completes in the guest.
- **Result: `ok` — 1 passed; 0 failed; finished in 63.02 s** (includes warm wait of
  60 s + both executions). No QEMU orphans left behind.

## 2. `promotion_latency` (the M4 "400 ms" number)

- Test: `crates/sandbox-promote/tests/promotion_latency.rs`, run release under HVF with
  a 1-slot warm `VmPool`.
- Timings (`pool.take()` + `promote_to_vm` per workspace shape):

| Workspace | Files | Bytes | Total ms | Warm handout ms |
|-----------|-------|-------|----------|-----------------|
| empty     | 0     | 0     | **0**    | 0               |
| small_8x4KiB | 8  | 32,768| **9**    | 0               |
| medium_200x8KiB | 200 | 1,638,400 | **261** | 0        |
| large_1x32MiB | 1   | 33,554,432| **644** | 0          |

- The 400 ms assertion applies to the small workspace: **9 ms ≪ 400 ms → PASS.**
- Warm handout from a pre-filled pool is 0 ms (the pool was refilled between shapes).
- Cost at larger trees is on record: ~1.3 ms/file up to 200 files; a single 32 MiB file
  dominates at 644 ms (copy-bound, as designed).

## Gap / notes
- Both acceptance lines are satisfied: "ffmpeg and headless Chromium run via
  auto-promotion" (real binaries, real guest) and "Promotion under 400 ms" (9 ms small).
- Environment caveat: the test is `#[ignore]`-gated on `SANDBOX_VM_IMAGE` and needs a
  built guest image; the Docker build script works on this Mac (Apple Silicon, HVF).
- Remaining M4 lines (VM resume < 150 ms, 24 h guest-protocol fuzz) were **not in scope**
  for this gap item and are not claimed here.
- Changes on the autopromote branch (`ffa209f`): pool pre-fill in `api.rs`, HVF/
  `GuestArch aarch64` support in `sandbox-vmm`, the two tests, and the guest-image
  Docker build.
