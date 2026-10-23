# Security Model

**Status: pre-alpha. Nothing in this repository is a security boundary yet.** The layers below describe the intended design from plans/PLAN.md section 5. Each item is marked as:

- **Implemented**: code exists and is wired into a path that runs.
- **Partial**: code exists, but it is not enforced or not reachable from the public API.
- **Planned**: no code, or only a placeholder.

Do not run untrusted code on this codebase, and do not expose the server to untrusted clients. Session, exec, and filesystem handlers run against in-memory sessions. Snapshot and fork are not implemented. Requests need a valid API key, but key provisioning, TLS, and rate limiting are not implemented (see Authentication).

## Assumptions

These assumptions are explicit so they can be checked as the code changes.

1. **Single-tenant development only today.** No tenant isolation is enforced. The `TenantId` field exists in `SessionConfig`, but nothing uses it for isolation.
2. **Isolates are not a security boundary on their own.** PLAN.md invariant 5 says one OS process per tenant. `sandbox-worker` runs each session in its own child process, and on Linux that process has namespaces, a seccomp denylist, `NO_NEW_PRIVS`, and cgroup v2 limits (ADR 0022). The denylist allows any syscall it does not name. The layers are not tested against a hostile workload, and the Linux-only layers do not exist on macOS.
3. **Untrusted code is assumed to attempt host access, network exfiltration, resource exhaustion, and sandbox escape.** Each layer below is designed against those attempts.
4. **The host operating system and Firecracker/KVM are trusted** in the design. Tier 2 would rely on them; Tier 2 is not implemented.
5. **Dependencies (V8 via `deno_core`, `wasmtime`) are trusted only after review and version pinning.** The workspace pins versions in `Cargo.toml`, but no CVE tracking or automated audit runs in CI.

## Defense in depth

Layers, outermost first (plans/PLAN.md section 5):

| # | Layer | Status | Where |
|---|-------|--------|-------|
| 1 | Hardware boundary: Firecracker + jailer | Planned | `crates/sandbox-vmm/src/jailer.rs` returns `unsupported` from `Jailer::setup` |
| 2 | Per-tenant OS process: seccomp, cgroups, namespaces, `NO_NEW_PRIVS` | Partial (Linux) | `crates/sandbox-worker/src/isolation.rs` applies namespaces and a seccomp denylist in the child, and sets cgroup v2 limits from the host. Each layer fails closed. Tests run on Linux (`cli/tests/worker_isolation.rs`). Off Linux, only the unisolated worker starts. |
| 3 | V8 isolate / wasmtime store: memory and CPU caps | Partial | V8: wall-clock watchdog, heap cap, fresh isolate per run. WASM: fuel and a linear-memory cap per run. |
| 4 | Capability operations: each host access checks policy | Partial | Policy helpers in `crates/sandbox-policy`; not called from runtime, shell, or server |

## Layer 1: Hardware boundary (Tier 2, planned)

Planned: Firecracker microVM per session, launched by a jailer (chroot, cgroups v2, network namespace, seccomp, dropped capabilities). Requires `/dev/kvm`.

Current state: `crates/sandbox-vmm` has driver, jailer, and pool scaffolding. `Jailer::setup` returns an unsupported error. `crates/sandbox-promote` returns an unsupported error. No VM is started anywhere.

## Layer 2: Per-tenant process (planned)

Built on Linux: one worker process per session, with `NO_NEW_PRIVS` and a seccomp-bpf denylist (`seccompiler`), namespaces (network, IPC, UTS, and a private mount tree), and cgroup v2 limits set from the host. The host needs a delegated cgroup in `SANDBOX_CGROUP_ROOT` (ADR 0022). An allowlist is planned next.

Current state: `crates/sandbox-worker/src/worker.rs` spawns the `sandbox worker --socket` subcommand as a child process with the isolation flags, and serves gRPC over a Unix socket in a directory the host creates with mode `0700`. Guest output is data in the reply, not a shared stream. A crashed child returns `Unavailable` to the host. `crates/sandbox-worker/src/isolation.rs` holds the namespace, seccomp, and cgroup code.

## Layer 3: Language runtimes (planned)

### V8 isolates (Tier 1)

Planned: heap limit through `CreateParams`, a near-heap-limit callback, a CPU watchdog that calls `terminate_execution`, a wall timeout, a cap on pending ops, and a fresh context per tenant execution.

Current state: `crates/sandbox-runtime/src/runtime.rs` runs each script in a fresh `deno_core` isolate.

**`node` is contained.** The shell's `node` builtin runs each script in a child process, through `crates/sandbox-worker/src/script.rs`. The child gets namespaces and a seccomp denylist, and the host moves it into a cgroup with `memory.max` set to the heap limit plus 128 MiB, and `memory.swap.max` set to 0. A script that exhausts that memory is killed by the kernel, and the host reports it as a script failure. A fatal V8 out-of-memory in the child cannot take down the host. Without a contained runner, `node` is refused (`ErrorCode::Unsupported`). A runner is installed on Linux only, with `FRONTAL_SANDBOX_UNSAFE_IN_PROCESS_SCRIPTS=1` as the only way to run scripts in-process, and that is for development only. A worker process is already isolated, so `node` inside a worker runs in that process, under the worker's cgroup cap.

The in-process path (`execute_script`) still has the limits described above: a fatal V8 out-of-memory aborts the host, and string growth is not bounded by the heap limit. Hosts must not use it for untrusted code. A watchdog terminates the isolate after `ScriptLimits::timeout`, and output is capped at `max_output_bytes`. The heap is capped at `max_heap_bytes` by V8 heap limits, and a near-limit callback terminates the script before V8 would abort the process. The only op is `op_sandbox_print`, which writes captured output. There is no filesystem, environment, or network access. Tests check that `Deno`, `process`, and `require` are not host capabilities.

### Wasmtime (Tier 1)

Planned: pooling allocator, epoch interruption, and a persistent AOT cache with integrity checks.

Current state: `crates/sandbox-wasm/src/wasi.rs` runs WASI preview1 command modules. Stdio is in memory, argv is the only input, and no environment is inherited. The only directory the guest can reach is an optional VFS preopen, and guest paths that leave it are refused. CPU is limited by fuel, and linear memory is capped at `max_memory_bytes` through `StoreLimits`. Tools named in `/etc/tools.toml` can pin a BLAKE3 hash, checked before each run.

## Layer 4: Capability operations

Target: only the operations in the table below exist, and each one checks a `Capability` against the session's policy and emits a trace event on denial.

### Capability types

Defined in `crates/sandbox-types/src/policy.rs` (`Capability`):

```
FsRead(path)     FsWrite(path)    NetHttp(host)     NetTcp(host:port)
Exec(tool)       Env(name)        Clock
```

### Known gaps in the current code

These are defects or gaps found while writing this page. Each one should be fixed or tracked before the operation is used.

- **Ambient environment access.** Removed. The earlier `op_env_get`/`op_env_set` ops read and wrote the host process environment, which violated plans/PLAN.md invariant 3. They were deleted rather than wired up. Any future host capability needs a policy check.
- **Filesystem capabilities use the network check.** `check_fs_read` and `check_fs_write` in `crates/sandbox-policy/src/capability.rs` call `policy.check_http`. Filesystem access is not governed by the network policy.
- **`Exec`, `Env`, and `Clock` always allow.** `check_capability` returns `true` for these.
- **Wildcard host matching is loose.** In `crates/sandbox-types/src/policy.rs`, a pattern `*.example.com` matches any host ending in `example.com`, including `evilexample.com`, because there is no dot boundary. The second comparison, `host == &suffix[1..]`, compares against `xample.com`, which is wrong. Use a dot-anchored suffix match and test both cases.
- **DNS is resolved but not pinned.** `resolve_and_check_host` in `crates/sandbox-policy/src/capability.rs` checks resolved addresses for private ranges, but its result is not used for the connection, so DNS rebinding is not prevented. No caller exists in the repository.
- **Private-address check is incomplete.** `is_private_ip` covers IPv4 loopback, RFC 1918, link-local (which includes the `169.254.169.254` metadata address), broadcast, and documentation ranges, and IPv6 loopback, link-local, and unique-local. It does not cover `0.0.0.0/8`, the carrier-grade NAT range `100.64.0.0/10`, or IPv4-mapped IPv6 addresses. It is called only from `resolve_and_check_host`, which has no caller.

### Network egress

Current state (types only):

- `NetworkPolicy::default()` is default-deny with no rules (`crates/sandbox-types/src/policy.rs`). `SessionConfig::default()` uses it.
- `NetworkPolicy::allow_all()` disables the check. It is available to callers and is not restricted.
- Helper policy sets for GitHub, npm, PyPI, and Docker are in `crates/sandbox-policy/src/network.rs`.

Planned (plans/PLAN.md section 4.7): an in-process HTTP client that is the only egress path, with pinned DNS, size and time limits, private and metadata range blocks, and request recording. None of this exists. `op_fetch` in `crates/sandbox-runtime/src/ops.rs` is a placeholder that returns an empty string.

### Secrets injection (planned)

`SecretsConfig` and `SecretEntry` in `crates/sandbox-types/src/policy.rs` describe per-host secrets. No egress proxy attaches them, and nothing reads them. The design goal is that agent code never sees raw tokens. Until the proxy exists, do not put real credentials in `SessionConfig.secrets` or `SessionConfig.env`.

## Resource limits

`ResourceLimits` and the `check_*` helpers exist, but nothing calls the helpers, and VFS quotas are not enforced. See [LIMITS.md](LIMITS.md) for values and status. Until enforcement exists, a session has no effective memory, CPU, storage, or inode cap.

## Authentication and authorization

- `crates/sandbox-server/src/auth.rs`: keys are `sbx_<id>_<secret>` with 256-bit secrets from the OS RNG. Only a BLAKE3 digest of the secret is stored, and comparison uses `blake3::Hash` equality. Keys can be revoked by id.
- `crates/sandbox-server/src/api.rs`: `require_api_key` is a `route_layer` on every `/v1` route. It returns 401 unless the bearer key validates.
- Still planned (plans/PLAN.md section 4.15): per-key scopes, quotas, key provisioning, and OIDC.
- `crates/sandbox-server/src/middleware.rs` adds a request ID and logs requests. It does not authenticate them.
- Settings the backend does not enforce are refused, not ignored. The crate has no server binary yet.

## Trace and audit

- `crates/sandbox-types/src/event.rs` defines the event schema, including `PolicyDenied`, `NetRequest`, `ExecStarted`, and `FileChanged`.
- Nothing emits these events yet. `PolicyDenied` has no emitter in the repository.
- Planned: append-only JSONL per session with monotonic sequence numbers (plans/PLAN.md section 4.12). The plan does not require tamper evidence, and the current design does not provide it. Do not describe the trace as tamper-evident.

## Dependency and supply-chain controls

- `cargo deny check` (licenses, advisories, bans, sources) is configured by `deny.toml`. Permissive licenses are allowed, and GPL and AGPL are refused. An advisory is ignored only when `deny.toml` gives a reason.
- `cargo audit` reads `.cargo/audit.toml`, which carries the same advisory decisions.
- Both run in the `supply-chain` job of `.github/workflows/ci.yml`, with pinned tool versions. They were run locally on 2026-10-19 and pass. The GitHub job has not been observed.
- `unsafe` policy: allowed only in named modules with a `// SAFETY:` comment and a test (PLAN.md section 5). Not enforced by a lint in the repository.
- `unwrap()`: PLAN.md section 11 forbids it outside tests and `main`. Not enforced by a lint in the repository.

## Required before any public launch

PLAN.md section 5 lists these. Some are done, and the rest are open.

- [x] Threat model document: `docs/THREAT_MODEL.md`
- [ ] External security review of the worker and VMM
- [x] Fuzz targets: shell parser, VFS path handling, WASI fs shim, guest protocol decoder (`cargo-fuzz`). All four are in `fuzz/` (see DEVELOPMENT.md). The guest protocol target covers the wire frames and the generated messages; the guest agent itself does not exist yet, and the 24 h run that PLAN §9 M4 asks for has not been done
- [ ] Escape tests: host file reads, metadata IP reachability, memory/CPU/inode/PID exhaustion, fork-bomb equivalents, symlink races, zip bombs
- [ ] Never share a V8 heap or wasmtime store across tenants
- [ ] Pin and track V8, wasmtime, and Firecracker CVEs; automate dependency updates
- [x] `cargo-deny` and `cargo-audit` in CI (the `supply-chain` job; see above)

## Reporting

No security reporting process or contact is defined in this repository yet. Do not file public issues for suspected sandbox escapes until one exists.
