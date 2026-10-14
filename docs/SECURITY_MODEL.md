# Security Model

**Status: pre-alpha. Nothing in this repository is a security boundary yet.** The layers below describe the intended design from PLAN.md section 5. Each item is marked as:

- **Implemented**: code exists and is wired into a path that runs.
- **Partial**: code exists, but it is not enforced or not reachable from the public API.
- **Planned**: no code, or only a placeholder.

Do not run untrusted code on this codebase, and do not expose the server to untrusted clients. The API handlers are placeholders. Requests need a valid API key, but key provisioning, TLS, and rate limiting are not implemented (see Authentication).

## Assumptions

These assumptions are explicit so they can be checked as the code changes.

1. **Single-tenant development only today.** No tenant isolation is enforced. The `TenantId` field exists in `SessionConfig`, but nothing uses it for isolation.
2. **Isolates are not a security boundary on their own.** PLAN.md invariant 5 says one OS process per tenant. `sandbox-worker` runs each session in its own child process, but without seccomp, cgroups, or namespaces. It isolates crashes, not hostile code.
3. **Untrusted code is assumed to attempt host access, network exfiltration, resource exhaustion, and sandbox escape.** Each layer below is designed against those attempts.
4. **The host operating system and Firecracker/KVM are trusted** in the design. Tier 2 would rely on them; Tier 2 is not implemented.
5. **Dependencies (V8 via `deno_core`, `wasmtime`) are trusted only after review and version pinning.** The workspace pins versions in `Cargo.toml`, but no CVE tracking or automated audit runs in CI.

## Defense in depth

Layers, outermost first (PLAN.md section 5):

| # | Layer | Status | Where |
|---|-------|--------|-------|
| 1 | Hardware boundary: Firecracker + jailer | Planned | `crates/sandbox-vmm/src/jailer.rs` returns `unsupported` from `Jailer::setup` |
| 2 | Per-tenant OS process: seccomp, cgroups, namespaces, `NO_NEW_PRIVS` | Partial | `crates/sandbox-worker/src/worker.rs` runs one child process per session with a JSON protocol and crash isolation. Seccomp, cgroups, namespaces, and `NO_NEW_PRIVS` are not implemented (Linux-only). |
| 3 | V8 isolate / wasmtime store: memory and CPU caps | Partial | V8: wall-clock watchdog, heap cap, fresh isolate per run. WASM: fuel and a linear-memory cap per run. |
| 4 | Capability operations: each host access checks policy | Partial | Policy helpers in `crates/sandbox-policy`; not called from runtime, shell, or server |

## Layer 1: Hardware boundary (Tier 2, planned)

Planned: Firecracker microVM per session, launched by a jailer (chroot, cgroups v2, network namespace, seccomp, dropped capabilities). Requires `/dev/kvm`.

Current state: `crates/sandbox-vmm` has driver, jailer, and pool scaffolding. `Jailer::setup` returns an unsupported error. `crates/sandbox-promote` returns an unsupported error. No VM is started anywhere.

## Layer 2: Per-tenant process (planned)

Planned: one worker process per tenant, with `prctl(PR_SET_NO_NEW_PRIVS)`, a seccomp-bpf allowlist (`seccompiler`), cgroup v2 limits, optional namespaces (`rustix`/`nix`), and no host mounts.

Current state: `crates/sandbox-worker/src/worker.rs` spawns the `sandbox worker` subcommand as a child process and talks to it over stdin and stdout. A crashed child returns `Unavailable` to the host. No seccomp, cgroup, or namespace code exists in the repository. A grep for `seccomp`, `cgroup`, and `prctl` in `crates/` finds no implementation.

## Layer 3: Language runtimes (planned)

### V8 isolates (Tier 1)

Planned: heap limit through `CreateParams`, a near-heap-limit callback, a CPU watchdog that calls `terminate_execution`, a wall timeout, a cap on pending ops, and a fresh context per tenant execution.

Current state: `crates/sandbox-runtime/src/runtime.rs` runs each script in a fresh `deno_core` isolate. A watchdog terminates the isolate after `ScriptLimits::timeout`, and output is capped at `max_output_bytes`. The heap is capped at `max_heap_bytes` by V8 heap limits, and a near-limit callback terminates the script before V8 would abort the process. The only op is `op_sandbox_print`, which writes captured output. There is no filesystem, environment, or network access. Tests check that `Deno`, `process`, and `require` are not host capabilities.

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

- **Ambient environment access.** Removed. The earlier `op_env_get`/`op_env_set` ops read and wrote the host process environment, which violated PLAN.md invariant 3. They were deleted rather than wired up. Any future host capability needs a policy check.
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

Planned (PLAN.md section 4.7): an in-process HTTP client that is the only egress path, with pinned DNS, size and time limits, private and metadata range blocks, and request recording. None of this exists. `op_fetch` in `crates/sandbox-runtime/src/ops.rs` is a placeholder that returns an empty string.

### Secrets injection (planned)

`SecretsConfig` and `SecretEntry` in `crates/sandbox-types/src/policy.rs` describe per-host secrets. No egress proxy attaches them, and nothing reads them. The design goal is that agent code never sees raw tokens. Until the proxy exists, do not put real credentials in `SessionConfig.secrets` or `SessionConfig.env`.

## Resource limits

`ResourceLimits` and the `check_*` helpers exist, but nothing calls the helpers, and VFS quotas are not enforced. See [LIMITS.md](LIMITS.md) for values and status. Until enforcement exists, a session has no effective memory, CPU, storage, or inode cap.

## Authentication and authorization

- `crates/sandbox-server/src/auth.rs`: keys are `sbx_<id>_<secret>` with 256-bit secrets from the OS RNG. Only a BLAKE3 digest of the secret is stored, and comparison uses `blake3::Hash` equality. Keys can be revoked by id.
- `crates/sandbox-server/src/api.rs`: `require_api_key` is a `route_layer` on every `/v1` route. It returns 401 unless the bearer key validates.
- Still planned (PLAN.md section 4.15): per-key scopes, quotas, key provisioning, and OIDC.
- `crates/sandbox-server/src/middleware.rs` adds a request ID and logs requests. It does not authenticate them.
- The routes are placeholders that return 501, and the crate has no server binary yet.

## Trace and audit

- `crates/sandbox-types/src/event.rs` defines the event schema, including `PolicyDenied`, `NetRequest`, `ExecStarted`, and `FileChanged`.
- Nothing emits these events yet. `PolicyDenied` has no emitter in the repository.
- Planned: append-only JSONL per session with monotonic sequence numbers (PLAN.md section 4.12). The plan does not require tamper evidence, and the current design does not provide it. Do not describe the trace as tamper-evident.

## Dependency and supply-chain controls

- Planned: `cargo-deny` (licenses and advisories) and `cargo-audit` in CI. Neither runs in `.github/workflows/ci.yml` or `pr-checks.yml`, and there is no `deny.toml` in the repository.
- `unsafe` policy: allowed only in named modules with a `// SAFETY:` comment and a test (PLAN.md section 5). Not enforced by a lint in the repository.
- `unwrap()`: PLAN.md section 11 forbids it outside tests and `main`. Not enforced by a lint in the repository.

## Required before any public launch

PLAN.md section 5 lists these. None are done.

- [ ] Threat model document (this page is not one)
- [ ] External security review of the worker and VMM
- [ ] Fuzz targets: shell parser, VFS path handling, WASI fs shim, guest protocol decoder (`cargo-fuzz`)
- [ ] Escape tests: host file reads, metadata IP reachability, memory/CPU/inode/PID exhaustion, fork-bomb equivalents, symlink races, zip bombs
- [ ] Never share a V8 heap or wasmtime store across tenants
- [ ] Pin and track V8, wasmtime, and Firecracker CVEs; automate dependency updates
- [ ] `cargo-deny` and `cargo-audit` in CI

## Reporting

No security reporting process or contact is defined in this repository yet. Do not file public issues for suspected sandbox escapes until one exists.
