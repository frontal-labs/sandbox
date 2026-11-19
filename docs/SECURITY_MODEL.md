# Security Model

**Status: pre-alpha. Nothing in this repository is a security boundary yet.** No external security review has been performed — see [SECURITY_REVIEW.md](SECURITY_REVIEW.md). The layers below describe the intended design from the project specification section 5. Each item is marked as:

- **Implemented**: code exists and is wired into a path that runs.
- **Partial**: code exists, but it is not enforced or not reachable from the public API.
- **Planned**: no code, or only a placeholder.

Do not run untrusted code on this codebase, and do not expose the server to untrusted clients. Session, exec, and filesystem handlers run against in-memory sessions. Requests need a valid API key, but key provisioning, TLS, and rate limiting are not implemented (see Authentication).

## Assumptions

These assumptions are explicit so they can be checked as the code changes.

1. **Single-tenant development only today.** No tenant isolation is enforced. The `TenantId` field exists in `SessionConfig`, but nothing uses it for isolation.
2. **Isolates are not a security boundary on their own.** The project specification invariant 5 says one OS process per tenant. `sandbox-worker` runs each session in its own child process, and on Linux that process has namespaces, a seccomp denylist, `NO_NEW_PRIVS`, and cgroup v2 limits (ADR 0022). The denylist allows any syscall it does not name. The layers are not tested against a hostile workload, and the Linux-only layers do not exist on macOS.
3. **Untrusted code is assumed to attempt host access, network exfiltration, resource exhaustion, and sandbox escape.** Each layer below is designed against those attempts.
4. **The host operating system and QEMU/KVM are trusted** in the design. Tier 2 relies on them when enabled; QEMU with KVM is the hardware boundary, and `Accel::Tcg` is not.
5. **Dependencies (V8 via `deno_core`, `wasmtime`) are trusted only after review and version pinning.** The workspace pins versions in `Cargo.toml`, but no CVE tracking or automated audit runs in CI.

## Defense in depth

Layers, outermost first (project specification section 5):

| # | Layer | Status | Where |
|---|-------|--------|-------|
| 1 | Hardware boundary: QEMU microVM (KVM or TCG) | Partial | `crates/sandbox-vmm/src/qemu.rs` boots a real guest; `sandbox-promote::promote_to_vm` is used for `vm`-tier sessions; not the default tier, and TCG is not a boundary |
| 2 | Per-tenant OS process: seccomp, cgroups, namespaces, `NO_NEW_PRIVS` | Partial (Linux) | `crates/sandbox-worker/src/isolation.rs` applies namespaces and a seccomp denylist in the child, and sets cgroup v2 limits from the host. Each layer fails closed. Tests run on Linux (`cli/tests/worker_isolation.rs`). Off Linux, only the unisolated worker starts. |
| 3 | V8 isolate / wasmtime store: memory and CPU caps | Partial | V8: wall-clock watchdog, heap cap, fresh isolate per run. WASM: fuel and a linear-memory cap per run. |
| 4 | Capability operations: each host access checks policy | Partial | Policy helpers in `crates/sandbox-policy`; not called from runtime, shell, or server |

## Layer 1: Hardware boundary (Tier 2)

Planned: a microVM per session, launched by the VMM. KVM acceleration requires `/dev/kvm`; software emulation (`tcg`) works without it.

Current state: `crates/sandbox-vmm` runs QEMU as the Tier 2 driver (`QemuVm`, with `Accel::Tcg` or `Accel::Kvm`), waits for the guest agent before handing the guest out, and holds a warm pool (`VmPool`). `crates/sandbox-promote` implements `promote_to_vm`, and the server uses it for sessions created with the `vm` tier preference. The earlier Firecracker driver and jailer were scaffolding and were removed (T-05).

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

- **Ambient environment access.** Closed. The earlier `op_env_get`/`op_env_set` ops were deleted. A later op, `op_sandbox_env`, still returned the whole host process environment to guest code through the `node:` shim. It now returns an empty list, and `crates/sandbox-worker/src/script.rs` starts the contained script child with an empty environment (`env_clear`). A session's own environment is not wired into the runtime yet. Any future host capability needs a policy check.
- **Filesystem capabilities.** There is no filesystem path policy. `check_fs_read` and `check_fs_write` in `crates/sandbox-policy/src/capability.rs` used to call `policy.check_http`, which judged paths by network host rules. They now deny every request, so nothing can rely on them by mistake. The VFS and its quotas are what bound filesystem access today.
- **`Exec`, `Env`, and `Clock`.** `check_capability` used to return `true` for these. The policy has no model for them, so they are now denied until one exists. Nothing in the repository calls `check_capability`.
- **Wildcard host matching.** Fixed. A pattern `*.example.com` matched `evilexample.com` because there was no dot boundary, and a `*.` pattern with a multibyte suffix could panic. Matching now compares on a label boundary, ignores case and a trailing dot, and treats `*.example.com` as subdomains only. The apex `example.com` needs its own rule. This is stricter than before.
- **Deny rules.** Fixed. They were ignored: `allow *` plus `deny evil.com` allowed `evil.com`. A matching deny rule now always wins, including when `default_deny` is off.
- **DNS is resolved but not pinned.** `resolve_and_check_host` returns the address it resolved, and the caller must connect to that address and not resolve the name again, or DNS rebinding is not prevented. No caller exists in the repository. The policy is now checked before any lookup, so a denied host causes no DNS query.
- **Private-address check.** `is_private_ip` now also covers `0.0.0.0/8`, `100.64.0.0/10`, `192.0.0.0/24`, `198.18.0.0/15`, multicast, `240.0.0.0/4`, IPv4-mapped and IPv4-compatible IPv6, NAT64 (`64:ff9b::/96`), 6to4 (`2002::/16`), site-local, and IPv6 multicast and documentation ranges.
- **An `allow_http` rule also permits raw TCP.** Rules carry no protocol, so `check_tcp` accepts an HTTP rule that has no port list, on any port. Not changed, because it alters the rule semantics; it needs an ADR.

### Network egress

Current state (types only):

- `NetworkPolicy::default()` is default-deny with no rules (`crates/sandbox-types/src/policy.rs`). `SessionConfig::default()` uses it.
- `NetworkPolicy::allow_all()` disables the check. It is available to callers and is not restricted.
- Helper policy sets for GitHub, npm, PyPI, and Docker are in `crates/sandbox-policy/src/network.rs`.

Planned (project specification section 4.7): an in-process HTTP client that is the only egress path, with pinned DNS, size and time limits, private and metadata range blocks, and request recording. None of this exists. `op_fetch` in `crates/sandbox-runtime/src/ops.rs` is a placeholder that returns an empty string.

### Secrets injection (planned)

`SecretsConfig` and `SecretEntry` in `crates/sandbox-types/src/policy.rs` describe per-host secrets. No egress proxy attaches them, and nothing reads them. The design goal is that agent code never sees raw tokens. Until the proxy exists, do not put real credentials in `SessionConfig.secrets` or `SessionConfig.env`.

## Resource limits

`ResourceLimits` and the `check_*` helpers exist, but nothing calls the helpers, and VFS quotas are not enforced. See [LIMITS.md](LIMITS.md) for values and status. Until enforcement exists, a session has no effective memory, CPU, storage, or inode cap.

## Authentication and authorization

- `crates/sandbox-server/src/auth.rs`: keys are `sbx_<id>_<secret>` with 256-bit secrets and 64-bit ids drawn from the OS random source (through the `uuid` v4 generator, whose fixed version bits are skipped). Until this refresh they came from a clock-seeded counter, which was predictable, so keys minted by an earlier build should be treated as compromised and replaced. If the OS source is unavailable, key creation panics instead of using a weak source. Only a BLAKE3 digest of the secret is stored, and comparison uses `blake3::Hash` equality. `AuthManager::revoke` exists, but no route exposes it, so a leaked key cannot be revoked without wiping the stored key state.
- `crates/sandbox-server/src/api.rs`: `require_api_key` is a `route_layer` on every `/v1` route. It returns 401 unless the bearer key validates.
- Still planned (project specification section 4.15): per-key scopes, quotas, key provisioning, and OIDC.
- `crates/sandbox-server/src/middleware.rs` adds a request ID and logs requests. It does not authenticate them.
- The server listens in plaintext. `[server.tls]` and `[server.auth]` in the TOML config (ADR 0036) are accepted but not implemented, and loading a file that sets `tls.enabled = true` or any `[server.auth]` value logs a warning. The default bind address is `0.0.0.0:8080`. Put the server behind a TLS-terminating proxy and do not expose it directly.
- `/v1/gc` and `/metrics` need only some valid key. Any tenant can trigger the global collection, which takes every session's gate, at most once a minute. There is no admin role yet.

## Trace and audit

- `crates/sandbox-types/src/event.rs` defines the event schema, including `PolicyDenied`, `NetRequest`, `ExecStarted`, and `FileChanged`.
- Nothing emits these events yet. `PolicyDenied` has no emitter in the repository.
- Planned: append-only JSONL per session with monotonic sequence numbers (project specification section 4.12). The plan does not require tamper evidence, and the current design does not provide it. Do not describe the trace as tamper-evident.

## Dependency and supply-chain controls

- `cargo deny check` (licenses, advisories, bans, sources) is configured by `deny.toml`. Permissive licenses are allowed, and GPL and AGPL are refused. An advisory is ignored only when `deny.toml` gives a reason.
- `cargo audit` reads `.cargo/audit.toml`, which carries the same advisory decisions.
- Both run in the `supply-chain` job of `.github/workflows/ci.yml`, with pinned tool versions. They were run locally on 2026-10-19 and pass, and the GitHub job runs green on every pull request and main push (confirmed 2026-11-14). The KVM VM workflow (`.github/workflows/vm-kvm.yml`) also runs green on main pushes and pull requests, executing the full `--ignored` VM test suites with hardware virtualisation.
- `unsafe` policy: allowed only in named modules with a `// SAFETY:` comment and a test (project specification section 5). Not enforced by a lint in the repository.
- `unwrap()`: project specification section 11 forbids it outside tests and `main`. Not enforced by a lint in the repository.

## Required before any public launch

The project specification section 5 lists these. Some are done, and the rest are open.

- [x] Threat model document: `docs/THREAT_MODEL.md`
- [ ] External security review of the worker and VMM — **not yet performed** (see [SECURITY_REVIEW.md](SECURITY_REVIEW.md))
- [x] Fuzz targets: shell parser, VFS path handling, WASI fs shim, guest protocol decoder (`cargo-fuzz`). All four are in `fuzz/` (see DEVELOPMENT.md). The guest protocol target covers the wire frames and the generated messages; the guest agent itself does not exist yet, and the 24 h run that PLAN §9 M4 asks for has not been done
- [ ] Escape tests: host file reads, metadata IP reachability, memory/CPU/inode/PID exhaustion, fork-bomb equivalents, symlink races, zip bombs
- [ ] Never share a V8 heap or wasmtime store across tenants
- [ ] Pin and track V8, wasmtime, and QEMU CVEs; automate dependency updates
- [x] `cargo-deny` and `cargo-audit` in CI (the `supply-chain` job; see above)

## Reporting

No security reporting process or contact is defined in this repository yet. Do not file public issues for suspected sandbox escapes until one exists.
