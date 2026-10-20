# Threat model

This is the threat model PLAN.md §5 requires before public launch. It describes
what the code does today, not what the plan intends. Where the two differ, the
difference is listed under "Residual risk". Controls are cited to the code that
enforces them, so a reviewer can check each claim. Last reviewed 2026-10-20.

## Assets

- **Tenant files and shell state.** The virtual filesystem, working directory,
  and environment of each session.
- **Session checkpoints and traces** kept by `serve --data-dir`.
- **API keys.** Only digests are stored (`crates/sandbox-server/src/auth.rs`).
- **The host process.** Today the isolate tier runs inside the server process
  (see Residual risk 1), so the host's memory, file descriptors, and CPU are
  assets too.

## Actors

- **Tenant client.** Holds an API key for one owner. Sends commands, reads and
  writes files, and takes snapshots. Assumed to be able to run arbitrary shell
  scripts, arbitrary WASI command modules that are already in the VFS, and
  arbitrary JavaScript through `node -e`.
- **Adversarial tenant code.** The scripts, JavaScript, and WASM modules a
  tenant runs. This is the main threat.
- **Unauthenticated network client.** Anyone who can reach the HTTP listener.
- **Other tenants.** Different API keys on the same server.
- **Operator.** Runs the server and holds the data directory. Trusted.

## Trust boundaries

1. **HTTP boundary.** Requests pass through API-key authentication and then
   owner checks before they reach any session (`crates/sandbox-server/src/api.rs`,
   `auth.rs`).
2. **Shell boundary.** Commands run in the interpreter over the VFS. The shell has
   no host filesystem access.
3. **V8 boundary.** `node -e` runs JavaScript in a `deno_core` isolate. The
   runtime exposes only `console.log` and the JavaScript built-ins.
4. **WASM boundary.** WASI command modules run in wasmtime, with fuel and memory
   limits. Their filesystem is at most one VFS directory (`crates/sandbox-wasm/src/wasi.rs`).
5. **Host boundary.** The process, its file descriptors, and the operating system.

## Controls, with where they are enforced

| Threat | Control | Where |
| --- | --- | --- |
| Unauthenticated access | API keys. Each key is a random secret; only its BLAKE3 digest is stored. Comparison is constant-time. | `auth.rs` |
| Cross-tenant access | Every session route checks that the key owns the session. A wrong owner gets 404, so existence is not revealed. | `api.rs` (`owned`) |
| Host file read or write from a script | The shell and filesystem have no host access. All paths resolve inside the VFS. `..` is refused, NUL bytes are refused, and symlink depth is capped. Fuzzed (`fuzz/fuzz_targets/vfs_paths.rs`). | `sandbox-vfs` |
| Host file access from WASM | No host preopens. At most one VFS directory is mapped. Rights bits are checked on open. | `sandbox-wasm/src/wasi.rs` |
| Network egress from tenant code | No network API exists in the runtime, shell, session, or WASI host. Nothing can open a socket. | whole tree (`grep` finds no socket or HTTP client in these crates) |
| Runaway loops in the shell | Step budget (1,000,000 commands and loop iterations per run), output budget (1 MiB), and a session wall timeout (300 s by default). | `sandbox-shell/src/interpreter.rs`, `sandbox-session/src/backend.rs` |
| Stack exhaustion from recursion or nesting | Command nesting capped at 32, substitution nesting at 32, parser depth bounded. Tests run on a 2 MiB stack. Found and fixed by fuzzing. | ADR 0018, `crates/sandbox-shell/tests/recursion.rs` |
| Pathological glob matching | Glob matching is memoized, so it is polynomial. Found by fuzzing. | `sandbox-shell/src/expand.rs`, `tests/globbing.rs` |
| Runaway JavaScript | 5 s wall-clock limit, 64 MiB heap limit, 1 MiB output cap, terminate-on-timeout. | `sandbox-runtime/src/runtime.rs` (`ScriptLimits`) |
| Runaway WASM | Fuel, a memory cap, and an output cap (`WasiLimits`). | `sandbox-wasm` |
| Resource exhaustion by storage | Per-session quotas on file size, total size, inode count, and path depth (ADR 0014). | `sandbox-vfs` quotas |
| Corrupted or swapped stored data | Blob reads hash the reassembled bytes and fail on mismatch (ADR 0017). | `sandbox-cas/src/disk_store.rs` |
| Unbounded trace growth | Trace budgets per session (`MAX_TRACE_RUNS`, `MAX_TRACE_BYTES`). | `api.rs` |
| Undetected GC of live data | GC marks from every live root and holds every session gate during a sweep (ADR 0013). | `sandbox-server/src/gc.rs` |
| Secrets in the durable store | The environment is kept in memory only, because its values are often secrets (ADR 0008, 0010). | `persist.rs` |
| Dependency vulnerabilities | `cargo deny` and `cargo audit` run in CI (`supply-chain` job). The advisory decisions and their reasons are in `deny.toml` and `.cargo/audit.toml`. | CI |

## Residual risk

These are not controls. Each one is a known gap, and some are high severity.
Launch is blocked on the ones marked **blocking**.

1. **Blocking. The isolate tier shares the server's process.** A memory-safety
   bug in the interpreter, the V8 binding, or wasmtime is a bug in the process
   that serves every tenant. A stack overflow aborts the process. Tier 1 is not
   a security boundary on its own (PLAN §2, invariant 5). The worker process
   with seccomp, cgroups, and namespaces (PLAN §4.13) is the boundary the plan
   requires. On Linux it is built (ADR 0022), with a denylist that is not yet an
   allowlist. Its layers are not yet tested against a hostile workload.
2. **Blocking. No tenant isolation of compute.** Memory, CPU, process, and
   open-file limits are refused with 501 rather than enforced
   (`ResourceLimits`, PLAN §14). A tenant can use CPU up to the wall timeout,
   and CPU time is not measured. Metering is wall time only.
3. **Blocking. No external security review** has been done (PLAN §5). This
   model is written by the team that built the code.
4. **A non-yielding command cannot be interrupted by the session timeout.** The
   504 comes at the deadline, but the V8 thread keeps running until the runtime's
   own 5 s limit stops it. A shell command that never awaits relies on the step
   budget.
5. **Side channels.** Timing of API-key comparison is constant-time, but the
   other paths (route lookup, session table) are not analysed.
6. **Denial of service by many requests or sessions.** There is no request rate
   limiter. The only 429 in the server is for GC triggers. Per-key session and
   snapshot counts are capped (ADR 0014), but nothing caps request rate. No
   measurement shows the server holds up under the 1,000-session target (PLAN §9
   M3). The load harness is in `benchmarks/load`, and no results are recorded.
7. **Crash-time data.** A crash can leave a temporary file in the chunk store.
   It is not swept, so it takes disk space (ADR 0017).
8. **Trace is not tamper-evident** (`docs/SECURITY_MODEL.md`). The plan does not
   require it.
9. **The network is absent by design, but the plan requires it later.** Egress
   through a policy-checked proxy with DNS pinning and blocked private ranges
   (PLAN §4.7) is not built. When it is, it needs its own threat analysis. The
   list of blocked ranges is part of the design, not an afterthought.
10. **Tier 2 (VM) is not built.** When it is, the guest protocol is untrusted
    input from inside a VM, and it needs its own fuzz target (PLAN §5, M4).

## Items that need an owner before launch

- Replace the seccomp denylist with an allowlist, measured under load, and test the layers against a hostile workload (ADR 0022).
- Enforce memory, CPU, process, and file-descriptor limits.
- Commission the external review, and record its findings.
- Run the escape test suite (PLAN §5). It is not written yet.
- Add fuzz targets for the WASI fs shim (via arbitrary modules) and the guest
  protocol decoder, once the guest exists.

## Reporting

No reporting process exists yet (see `docs/SECURITY_MODEL.md`). Do not file
public issues for suspected sandbox escapes.
