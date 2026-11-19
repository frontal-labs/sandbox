# M3 + M5 — Escape test suite / security hardening

**Verdict: PASS for the suites that exist (56 passed, 0 failed, 17 known-gap ignores);
the OS-level contained tier (namespaces/seccomp/cgroups) is UNBUILT — that is the gap.**

## 1. Suites built and passing

Worktree `t3-1aefc3d6`, commit `9694507`. Three escape suites were written (one per
execution tier) and run **natively** and inside a privileged Linux Docker container
(`frontal-sandbox-escape-security:latest`, built by `docker/escape-linux.Dockerfile`,
driven by `docker/run-escape-tests.sh`).

Linux container results (`/tmp/escape-linux.log`):

| Suite | Tests | Passed | Ignored (known gaps) | Failed |
|-------|-------|--------|----------------------|--------|
| WASM (`sandbox-wasm/tests/escape.rs`, 796 lines)   | 25 | 19 | 6 | 0 |
| SHELL (`sandbox-shell/tests/escape.rs`, 874 lines) | 27 | 24 | 3 | 0 |
| RUNTIME (`sandbox-runtime/tests/escape.rs`, 715 lines) | 21 | 13 | 8 | 0 |
| **Total** | **73** | **56** | **17** | **0** |

Representative passing checks:
- WASM: every kind of trap is a guest failure, not a host failure; `..`/absolute paths
  refused; stdio/unknown descriptors can't be opened as directories; output capped on
  both streams; extremely deep module nesting rejected without crashing the host.
- SHELL: symlinks resolve inside the VFS and cycles terminate; output floods stop at the
  cap; command substitution can't hold unbounded output; deep constructs don't overflow
  the stack; endless loops/recursion end.
- RUNTIME: a script cannot catch its own termination; the `node:` fs shim never returns
  host data; deep recursion/data are script errors; output flood capped on every
  channel; the watchdog stops every known spin/block vector.

The 17 ignores are explicit known gaps (e.g., memory-exhaustion probes that need a
resource controller, OS-level namespace checks that need Linux). They are ignored with
reasons, not silently skipped.

## 2. The unbuilt tier: OS-level containment (the real gap)

- The plan puts the memory-exhaustion and OS-level probes in
  `cli/tests/contained_escape.rs`, which **does not exist**. `cli/tests/` has no
  contained-escape target.
- `sandbox-worker/tests/` likewise has no `escape_linux.rs` (available targets:
  `isolation`, `worker`) — so the existing Docker script fails with
  `error: no test target named 'escape_linux' in sandbox-worker package` for that tier.
- `docs/ESCAPE_TESTS.md` is **missing**.
- Consistent with `MILESTONES.md` M1: "`sandbox-worker`: ... Seccomp, cgroups,
  namespaces, and gRPC are not implemented." The Rust-side escape suites are strong;
  the kernel-side containment they would defend is not there yet.

## Gap / notes
- "Escape test suite passes" (M3 acceptance) is satisfied for the suites that exist;
  the contained tier must be built (Linux + delegated cgroups) before the line can be
  claimed end-to-end.
- No existing tests were modified to make failures pass; all green results are from
  new tests against the current behavior.
- Re-run: `docker/run-escape-tests.sh` (privileged container, ~2.5 GB image).
