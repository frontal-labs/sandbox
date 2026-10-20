# Frontal Sandbox

*Instant, cheap sandboxes for AI agents: V8-fast shells and filesystems that escalate to real Linux microVMs only when needed.*

This document is the full spec for a coding agent to build `sandbox` in Rust. Work milestone by milestone. Do not start a milestone until the previous one's acceptance criteria pass.

## 1. Goals and non-goals

**Goals**

- Cold start of a Tier 1 session under 5 ms from a warm pool, under 25 ms from snapshot.
- Thousands of concurrent idle sessions per host; idle cost near zero.
- A shell and filesystem agents can use like a Linux box: `git`, `node`, `python`, `jq`, `grep`, `sed`, pipes, redirects.
- Snapshot and fork in O(1) using content-addressed storage.
- Automatic promotion to a Firecracker microVM when a command needs a real binary.
- A tiny API: `create`, `exec`, `fs`, `snapshot`, `fork`, `destroy`.
- Full trace of every command and file change, replayable.

**Non-goals (v1)**

- Windows or macOS guests.
- GPU workloads.
- Persisting running process state across sleep or promotion. Sessions keep their files and working directory across sleep and restart, but not their environment (its values are often secrets) or running processes.
- Full bash compatibility (target the 95% agents actually use).
- Multi-region replication.

## 2. Architecture overview

```
Client (Rust SDK / TS SDK / CLI)
   | HTTP + WebSocket
server  (auth, quotas, API)
   |
scheduler  (placement, warm pools, idle sleep, metering)
   |
worker process (one per tenant, seccomp + cgroups)
   |-- Session (trait)
        |-- IsolateBackend (Tier 1)
        |     |-- shell  (parser + interpreter + builtins)
        |     |-- runtime (deno_core V8 isolate)
        |     |-- wasm   (wasmtime + WASI tools)
        |     |-- vfs    (copy-on-write, content-addressed)
        |-- VmBackend (Tier 2)
              |-- vmm (Firecracker + guest agent)
snapshot store (object storage + local cache)
trace sink (append-only event log)
```

**Core invariants**

1. The VFS is the source of truth. Every backend mounts it. Promotion means "boot a VM with this VFS."
2. The SDK never knows which tier it is on. Tier is a property of the session, exposed read-only.
3. All host access (files, network, clock, env) goes through capability ops with a policy check. No ambient authority.
4. Every state change emits a trace event.
5. One OS process per tenant. Isolates are not a security boundary on their own.

## 3. Repository layout

Status: the workspace, crates, SDKs, and CLI below exist as scaffolds (see
`Cargo.toml`, `crates/`, `sdks/`, `cli/`). `deny.toml` and `fuzz/` exist. `tools/`,
`images/`, `tests/`, and `examples/` are planned and not yet present.

```
sandbox/
├── Cargo.toml                 # workspace
├── rust-toolchain.toml        # pin stable
├── deny.toml                  # cargo-deny: licenses, advisories, bans, sources
├── fuzz/                      # cargo-fuzz targets (nightly): shell, VFS paths
├── crates/
│   ├── sandbox-types/            # shared IDs, errors, event schema, config
│   ├── sandbox-vfs/              # virtual filesystem
│   ├── sandbox-cas/              # content-addressed blob store
│   ├── sandbox-shell/            # shell parser, interpreter, builtins
│   ├── sandbox-runtime/          # deno_core isolate host
│   ├── sandbox-wasm/             # wasmtime/WASI host
│   ├── sandbox-session/          # Session trait + IsolateBackend
│   ├── sandbox-snapshot/         # snapshot, fork, restore
│   ├── sandbox-vmm/              # Firecracker driver
│   ├── sandbox-guest/            # in-VM agent (static musl binary)
│   ├── sandbox-promote/          # Tier 1 to Tier 2 handoff
│   ├── sandbox-policy/           # capabilities, network policy, limits
│   ├── sandbox-trace/            # event log, diff, replay
│   ├── sandbox-worker/           # per-tenant process entrypoint
│   ├── sandbox-scheduler/        # placement, pools, idle, metering
│   └── sandbox-server/           # axum API
├── sdks/
│   ├── rust/                  # Rust client SDK (crate: sandbox-sdk)
│   ├── typescript/            # TS client SDK (npm: @frontal-labs/sandbox-sdk)
│   └── python/                # Python client SDK (package: sandbox_sdk)
├── cli/                       # `frontal-sandbox` binary (crate: cli)
├── benchmarks/                # criterion benches + load tests
├── tools/                     # build scripts: tools to WASM, rootfs images (planned)
├── images/                    # Tier 2 rootfs recipes (planned)
├── deploy/                    # Dockerfile, Terraform, systemd units (planned)
├── tests/                     # cross-crate integration + conformance (planned)
├── examples/                  # coding-agent, data-agent, browser-agent (planned)
└── docs/                      # architecture, security, API, limits
```

## 4. Crate specifications

### 4.1 `sandbox-types`

- `SessionId`, `SnapshotId`, `TenantId` (ULID newtypes), `ContentHash` (BLAKE3, 32 bytes).
- `sandboxError` (thiserror) with stable error codes surfaced to the API.
- `Event` enum for trace (see 4.12).
- `SessionConfig`: tier preference (`Auto | Isolate | Vm`), memory limit, CPU-ms budget, wall timeout, network policy, env, initial snapshot.
- Everything `serde` so it can be serialized. The OpenAPI spec is generated with
  `utoipa` (`ToSchema` behind the `openapi` feature, ADR 0001). `schemars`, which
  §4.15 also allows, is not used, so it is not a dependency.

### 4.2 `sandbox-cas`

Content-addressed blob store.

- Trait `BlobStore { get(hash), put(bytes) -> hash, has(hash) }`.
- Impls: `MemStore`, `DiskStore` (sharded dirs, mmap reads), `ObjectStore` (S3-compatible via the `object_store` crate), and `TieredStore` (local cache in front of remote).
- Chunking: FastCDC for files above 64 KiB, so edits dedupe. Small files stored whole.
- Compression: zstd. Hash is over uncompressed content.
- Refcount/GC via mark and sweep from live snapshot roots; GC never runs inline on the request path.

### 4.3 `sandbox-vfs`

Copy-on-write virtual filesystem.

- Layered model: immutable base (a tree rooted at a `ContentHash`) plus a mutable in-memory overlay. Reads check overlay then base. Writes go to overlay. Deletes are whiteouts.
- Tree nodes are Merkle objects: directories list `(name, mode, hash)`; files list chunk hashes. Root hash identifies the full state.
- POSIX-ish semantics: paths, symlinks, hard links (optional), permissions, mtimes (virtual clock), `rename` atomic, `O_APPEND`, `readdir` ordering stable.
- API (async): `open, read, write, stat, readdir, mkdir, rm, rename, symlink, readlink, truncate, watch`.
- `commit() -> ContentHash` flushes overlay into the CAS and returns the new root. Must be incremental: only dirty paths are rehashed.
- `export_ext4(root) -> block image` for Tier 2 promotion, and `import_from_dir` for the reverse (guest agent syncs back on demand).
- Quotas: total bytes, inode count, max file size, max path depth.
- Path safety: normalize, reject NUL, no escape above root, symlink resolution depth cap.
- Property tests (proptest) against a model filesystem. Fuzz the path normalizer.

### 4.4 `sandbox-shell`

A POSIX-ish shell that runs against the VFS, no host OS.

- **Parser:** start with `brush-parser` (Rust bash parser) or `tree-sitter-bash` behind our own AST. Own the AST so the parser is swappable.
- **Interpreter:** async, tokio-based. Supports pipes, `&&`, `||`, `;`, `&` (as in-runtime tasks), redirects (`>`, `>>`, `<`, `2>&1`, here-docs), subshells, variables, `export`, parameter expansion (`${x:-y}`, `${#x}`, `${x%pat}`), command substitution, globbing, `if/for/while/case`, functions, `set -e/-u/-o pipefail`, `trap` (EXIT only), `$?`, `$@`.
- **Process model:** a `Process` is an async task with stdin/stdout/stderr streams (bounded channels, backpressure), an exit code, and a cancellation token. No real PIDs; allocate virtual PIDs and a virtual `ps`.
- **Command resolution order:** function, builtin, VFS executable (shebang `#!/usr/bin/env node|python|sh`), PATH lookup in the tool registry (WASM tools), else return `NeedsVm { command }`.
- **Builtins:** `cd, pwd, echo, printf, test/[, export, unset, source, exit, read, type, which, true, false, sleep, wait, kill (virtual), env, alias`.
- **Coreutils:** use `uutils/coreutils` crates where they compile against an abstract FS trait; otherwise implement the top 40: `ls, cat, cp, mv, rm, mkdir, touch, head, tail, wc, sort, uniq, cut, tr, tee, find, xargs, grep (ripgrep core), sed, awk (subset), diff, patch, tar, gzip, base64, sha256sum, date, basename, dirname, realpath, stat, chmod, du, df`.
- Output must match GNU behavior for common flags. Every utility gets golden tests against real GNU output recorded in CI.
- Interactive PTY mode (line editing, ANSI) is M6, not v1.

### 4.5 `sandbox-runtime`

V8 host built on `deno_core`.

- Pin `deno_core` and `v8` versions exactly. Wrap them behind a small `JsRuntime` facade so upgrades touch one crate.
- Include only: `deno_webidl, deno_url, deno_web, deno_console, deno_crypto, deno_fetch`. Exclude `deno_fs, deno_net, deno_io, deno_node` initially.
- Custom ops, all capability-checked: `op_fs_*` (to VFS), `op_fetch` (to policy-checked HTTP client), `op_spawn` (to shell), `op_env`, `op_now` (virtual or real clock), `op_stdout/stderr`.
- **Startup snapshot:** build at compile time with `deno_core::snapshot` containing the bootstrap JS (Node-ish globals, `process`, `Buffer`, `require` shim). Target under 5 ms to a ready isolate from a pooled snapshot.
- **Node compatibility layer** (M2+): a `node:` module shim for `fs, path, os, child_process, events, stream, util, buffer, url, crypto` pointing at our ops. TypeScript via `swc` transpile on load.
- **Limits:** V8 heap limit via `CreateParams`, near-heap-limit callback that terminates cleanly, CPU watchdog thread calling `terminate_execution`, wall timeout, max op queue depth.
- **Pooling:** pre-warmed isolates keyed by bootstrap version; recycle by dropping and recreating the context, never reusing heap across tenants.

### 4.6 `sandbox-wasm`

WASI tool host on `wasmtime`.

- Component model + WASI preview 2 where available, preview 1 fallback.
- Custom WASI filesystem implementation backed by `sandbox-vfs` (never preopen host dirs).
- Pooling allocator, `epoch_interruption` for CPU limits, `StoreLimits` for memory, fuel optional for billing precision.
- Precompile modules to `.cwasm` at build time (AOT); store in CAS by hash; mmap at load.
- **Tool registry** (`tools.toml`): name, version, wasm hash, argv0, default limits. Tools to ship: `git` (try `gix`-based native Rust first, WASM build of libgit2 as fallback), `jq`, `ripgrep`, `sqlite3`, `python` (CPython WASI build), `curl`-like via our own fetch builtin, `zip/unzip`, `make` (optional).
- Python: two options, decide in M2 by benchmark: Pyodide inside V8, or CPython on WASI in wasmtime. Default to CPython WASI for isolation consistency; pre-initialize with Wizer for fast start.

### 4.7 `sandbox-policy`

- `Capability` set per session: `fs:read:<prefix>`, `fs:write:<prefix>`, `net:http:<host-pattern>`, `net:tcp:<host:port>`, `exec:<tool>`, `env:<name>`, `clock`.
- Network: default deny. Allowlist by host pattern. All egress through an in-process HTTP client that enforces: DNS resolution pinned (no rebinding), block private/link-local/metadata ranges (`169.254.0.0/16`, RFC1918, `::1`, etc.), size and time limits, optional request recording.
- Secrets injection: session config may hold secrets that the egress proxy attaches to matching hosts, so code never sees the raw token.
- Resource limits struct shared by all backends.
- Every deny emits a trace event.

### 4.8 `sandbox-session`

```rust
#[async_trait]
pub trait Session: Send + Sync {
    fn id(&self) -> SessionId;
    fn tier(&self) -> Tier;
    async fn exec(&self, req: ExecRequest) -> Result<ExecHandle>;
    fn fs(&self) -> &dyn SessionFs;
    async fn snapshot(&self) -> Result<SnapshotId>;
    async fn sleep(&self) -> Result<SnapshotId>;
    async fn destroy(self: Box<Self>) -> Result<()>;
}

pub struct ExecHandle {
    pub stdout: BoxStream<Bytes>,
    pub stderr: BoxStream<Bytes>,
    pub stdin: Sink<Bytes>,
    pub wait: JoinHandle<ExitStatus>,
    pub cancel: CancellationToken,
}
```

- `IsolateBackend` wires shell + runtime + wasm + vfs + policy.
- `VmBackend` proxies to the guest agent over vsock.
- `Auto` tier: start in Isolate; on `NeedsVm`, call `sandbox-promote`.

### 4.9 `sandbox-snapshot`

- `Snapshot { id, parent, vfs_root, env, cwd, tier, runtime_blob: Option<Hash>, created_at, labels }`, stored as a CAS object.
- `take(session)`: commit VFS, serialize shell state (env, cwd, functions, aliases), optionally V8 heap snapshot for warm isolate state.
- `restore(id) -> Session`: O(1) for the VFS (just mount the root hash), fast path for runtime blob.
- `fork(id, n) -> Vec<Session>`: n sessions sharing the same base, independent overlays.
- VM snapshots (Firecracker memory + disk) are a separate `VmSnapshot` kind, M5+.
- Snapshot retention policy and labels; GC integration with `sandbox-cas`.

### 4.10 `sandbox-vmm` and `sandbox-guest`

- Drive Firecracker over its Unix-socket REST API (`reqwest` + `hyperlocal` or hand-rolled hyper client). Jailer for each VM: chroot, cgroups, netns, seccomp, dropped caps.
- Base images: minimal Debian/Alpine rootfs plus `sandbox-guest`; separate `browser` image with Chromium + Playwright deps. Built reproducibly in `tools/`.
- Boot path: kernel + rootfs (read-only, shared across VMs) + per-session writable overlay block device populated from the VFS export.
- **`sandbox-guest`:** static musl binary, PID 1 or child of a tiny init. Listens on vsock. Protocol: length-prefixed protobuf or MessagePack frames: `Exec`, `Signal`, `ReadFile`, `WriteFile`, `ListDir`, `Sync(to host VFS)`, `Ping`. Streams stdout/stderr as frames.
- Networking: tap device per VM, nftables egress rules mirroring `sandbox-policy`, same private-range blocks.
- Fast resume: Firecracker snapshot/restore with userfaultfd lazy memory loading. Target under 150 ms.
- Warm pool of pre-booted VMs per image.

### 4.11 `sandbox-promote`

1. Pause the isolate session; `vfs.commit()`.
2. `export_ext4(root)` to a block image (cached by root hash).
3. Acquire a VM from the pool, attach the image.
4. Transfer env, cwd, limits. Re-run the failed command in the VM.
5. Swap the session's backend. Emit `Promoted` event. Reverse sync (VM to VFS) only on explicit `snapshot()` or sleep.

Be explicit in docs: running processes do not survive promotion. Include a `promote_hint` in the SDK so users can request Tier 2 up front.

### 4.12 `sandbox-trace`

- `Event` variants: `SessionCreated, ExecStarted{argv, cwd, env_hash}, ExecExited{code, cpu_ms, wall_ms}, StdoutChunk, StderrChunk, FileChanged{path, op, before_hash, after_hash}, NetRequest{host, method, status, bytes}, PolicyDenied, SnapshotTaken, Promoted, Slept, Woken`.
- Append-only log per session (JSONL locally; Parquet/ClickHouse sink optional). Ordered with monotonic sequence numbers.
- `diff(from_root, to_root)` yields file-level and line-level diffs.
- `replay(log, from_snapshot)` re-executes commands deterministically where possible (virtual clock, recorded network responses) and reports divergences.

### 4.13 `sandbox-worker`

- One process per tenant (can host many sessions of that tenant). Entry sets up: `prctl(PR_SET_NO_NEW_PRIVS)`, seccomp-bpf filter (via `seccompiler`), cgroup v2 limits, optional user + mount + pid + net namespaces (via `rustix`/`nix`), then listens on a Unix socket for scheduler RPC (`tonic` gRPC or `tarpc`).
- Crash isolation: the worker dies, the scheduler restores sessions from the last snapshot or reports failure.
- Hard rule: the worker never holds credentials for other tenants.

### 4.14 `sandbox-scheduler`

- Tracks hosts, workers, sessions. Placement by tenant affinity, then free memory.
- Warm pools: pre-spawned workers with a pre-warmed isolate; sized by recent demand.
- Idle policy: after N seconds without activity, snapshot, then free the worker. Wake on next request.
- Metering: CPU-ms (from epoch ticks or cgroup stats), memory GB-s, storage GB-month, egress bytes, VM seconds. Emit usage events to a durable queue.
- Metadata store: `redb` or SQLite for single-node; Postgres via `sqlx` for clustered mode. Abstract behind a `Meta` trait.
- Backpressure and admission control: reject with `429` plus `Retry-After` when over quota or capacity.

### 4.15 `sandbox-server`

`axum` + `tower` + `tracing`.

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/v1/sessions` | create (config, optional snapshot) |
| GET | `/v1/sessions/{id}` | status, tier, usage |
| DELETE | `/v1/sessions/{id}` | destroy |
| POST | `/v1/sessions/{id}/exec` | run command (streaming NDJSON or WebSocket) |
| GET | `/v1/sessions/{id}/ws` | interactive WebSocket (exec, stdin, signals) |
| GET/PUT/DELETE | `/v1/sessions/{id}/fs/{path}` | read/write/delete files |
| GET | `/v1/sessions/{id}/fs?list=...` | list directory |
| POST | `/v1/sessions/{id}/snapshot` | take snapshot |
| POST | `/v1/snapshots/{id}/fork` | fork n sessions |
| GET | `/v1/sessions/{id}/trace` | stream events |
| GET | `/v1/usage` | metering |

- Auth: API keys (hashed at rest, prefix-identified), per-key scopes and quotas. OIDC later.
- Generate an OpenAPI spec from types (`utoipa` or `schemars`); CI fails if the committed spec drifts.
- Rate limiting, request IDs, structured logs, Prometheus metrics, OpenTelemetry traces.

### 4.16 `sandbox-sdk` and `cli` (binary `frontal-sandbox`)

```rust
let client = sandbox::from_env()?;
let sb = client.create(SessionConfig::default().tier(Tier::Auto)).await?;
let out = sb.exec("git clone https://github.com/x/y && cd y && npm test").await?;
println!("{} {}", out.exit_code, out.stdout);
sb.fs().write("/work/a.txt", b"hi").await?;
let snap = sb.snapshot().await?;
let forks = snap.fork(3).await?;
```

- SDK also exposes a streaming `exec_stream`, `watch(path)`, and an in-process mode (`sandbox::embedded()`) that runs the isolate backend with no server, for tests and local use.
- CLI: `frontal-sandbox run <cmd>`, `frontal-sandbox shell` (interactive), `frontal-sandbox snapshot`, `frontal-sandbox fork`, `frontal-sandbox replay <trace>`, `frontal-sandbox serve`, `frontal-sandbox bench`.
- TS SDK generated from OpenAPI, then hand-polished for streaming.

## 5. Security model

**Layers, outermost first**

1. Hardware boundary (Tier 2 only): Firecracker + jailer.
2. Per-tenant OS process: seccomp allowlist, cgroup limits, namespaces, no host mounts, read-only binary.
3. V8 isolate / wasmtime store: memory and CPU caps.
4. Capability ops: only exposed ops exist; each checks policy.

**Required before any public launch**

- Threat model doc and an external security review of the worker and VMM.
- Fuzz targets: shell parser, VFS path handling, WASI fs shim, guest protocol decoder (`cargo-fuzz`).
- Escape tests: attempts to read host files, reach the metadata IP, exhaust memory/CPU/inodes/PIDs, fork-bomb equivalents, symlink races, zip-bombs.
- Never share a V8 heap or wasmtime store across tenants.
- Pin and track V8/wasmtime/Firecracker CVEs; automate dependency updates.
- `cargo-deny` and `cargo-audit` in CI; `unsafe` allowed only in named modules with comments.

## 6. Performance targets and benchmarks

| Metric | Target |
| --- | --- |
| Isolate session from warm pool | < 5 ms p50, < 15 ms p99 |
| Restore from snapshot (Tier 1) | < 25 ms p50 |
| `echo hi` end-to-end (local) | < 10 ms |
| VFS read 4 KiB (cached) | < 5 µs |
| Fork a snapshot | < 5 ms |
| Tier 2 resume from snapshot | < 150 ms p50 |
| Idle sessions per 64 GB host | 10,000+ |
| Idle memory per sleeping session | 0 (on disk only) |

Benchmarks live in `bench/`: `criterion` for micro (VFS, shell parse, op dispatch), a load tool for end-to-end (`k6` or a Rust driver), and a nightly CI job that fails on more than 10% regression.

## 7. Testing strategy

- **Unit and property tests** per crate (`proptest` for VFS, shell expansion, CAS).
- **Conformance suite** in `tests/`: a corpus of shell scripts run against both `frontal-sandbox` and real `bash`/GNU coreutils in a container, diffing stdout, stderr, exit code, and resulting file tree.
- **Agent transcript replay:** collect real agent command traces (Claude Code, Aider, SWE-agent style) and run them as regression tests. Track the pass rate as the headline compatibility metric.
- **Integration:** server + worker + SDK in-process with `tokio::test`; Tier 2 tests gated behind a feature and run on KVM-enabled CI runners.
- **Chaos:** kill workers mid-exec, corrupt a blob, fill disk; sessions must fail cleanly and recover from snapshot.
- **Determinism tests** for replay.

## 8. Dependencies (initial pin list)

`tokio`, `axum`, `tower`, `hyper`, `reqwest`, `tonic`, `serde`, `serde_json`, `schemars`, `utoipa`, `thiserror`, `anyhow` (bins only), `tracing`, `tracing-subscriber`, `opentelemetry`, `metrics-exporter-prometheus`, `clap`, `bytes`, `futures`, `async-trait`, `ulid`, `blake3`, `zstd`, `fastcdc`, `object_store`, `redb`, `sqlx`, `deno_core`, `deno_web`, `deno_fetch`, `deno_crypto`, `deno_url`, `swc_core`, `wasmtime`, `wasmtime-wasi`, `wizer`, `brush-parser`, `uutils` coreutils crates, `gix`, `seccompiler`, `rustix`, `nix`, `proptest`, `criterion`, `insta`, `cargo-fuzz`.

Dependency state (2026-10-19): `schemars` is not declared anywhere, because no code used it and the OpenAPI path is `utoipa` (§4.1). `wizer` is not declared either. It returns with M2 (Python pre-initialization), and then it must not pull wasmtime's default features, which include `profiling` (GPL `ittapi`). §14 records both decisions.

Rule: justify every new dependency in the PR description; prefer well-maintained crates with permissive licenses.

## 9. Milestones and acceptance criteria

### M0: Foundations (week 1)

- Cargo workspace, CI (fmt, clippy `-D warnings`, test, deny, audit), `sandbox-types`, `sandbox-cas` (Mem + Disk).
- **Accept:** CI green; CAS property tests pass.

### M1: Isolate + VFS + shell core (weeks 2-5)

- `sandbox-vfs` (overlay, commit, quotas), `sandbox-runtime` (deno\_core, startup snapshot, limits), `sandbox-shell` (parser, pipes, redirects, 20 builtins), `sandbox-session` IsolateBackend, `sandbox-sdk` embedded mode, `frontal-sandbox run`.
- **Accept:** `frontal-sandbox run "echo hi | cat > /a && cat /a"` works; `node -e` and a `.ts` script run against the VFS; cold start < 5 ms from warm pool in `bench/`; 200+ shell conformance cases pass.

### M2: WASM tools + Node compat + trace (weeks 6-9)

- `sandbox-wasm`, tool registry with `jq`, `ripgrep`, `sqlite3`, `git`, `python`; coreutils to 40 commands; `node:` shim; `sandbox-trace` with diffs and JSONL log.
- **Accept:** `git clone` (via allowed host), commit, diff works; `python -c` and `pip`-free stdlib scripts run; trace shows every file change; 70% pass rate on the first agent-transcript corpus.

### M3: Server, policy, snapshots (weeks 10-13)

- `sandbox-policy`, `sandbox-snapshot` (snapshot, restore, fork), `sandbox-worker` with seccomp/cgroups, `sandbox-server` API + OpenAPI, API keys, `sandbox-scheduler` (single-node), idle sleep/wake, metering.
- **Accept:** snapshot/fork under 5 ms; 1,000 concurrent sessions on one node in the load test; sleeping sessions use no memory; escape test suite passes; TS SDK generated.

### M4: Tier 2 (weeks 14-19)

- `sandbox-vmm`, `sandbox-guest`, base and browser images, `export_ext4`, `sandbox-promote`, VM warm pool, Firecracker snapshots.
- **Accept:** `ffmpeg` and headless Chromium run via auto-promotion; promotion under 400 ms; VM resume < 150 ms; guest protocol fuzzed for 24 h clean.

### M5: Production hardening (weeks 20-24)

- Clustered scheduler with Postgres, object-store CAS, GC, multi-tenant quotas, billing export, dashboards, runbooks, external security review.
- **Accept:** chaos suite passes; 10,000 idle sessions per host; documented SLOs; review findings resolved.

### M6: Developer experience (ongoing)

- Interactive PTY shell, `frontal-sandbox replay`, VS Code/MCP server exposing `exec` and `fs` as tools, docs site, examples, templates (snapshot images like "node-20", "python-3.12").

## 10. Observability and operations

- Metrics: session starts, start latency histogram, exec latency, promotion rate, policy denials, OOM kills, snapshot size, CAS hit rate, pool depth.
- Logs: structured JSON, request ID plus session ID on every line.
- Traces: OpenTelemetry spans across server, scheduler, worker.
- Admin: `sandbox admin` for draining a host, forcing GC, inspecting a session's trace.
- Deploy: single static binary + systemd unit for single node; container image; Terraform for cloud (bare metal or KVM-capable instances, since Firecracker needs `/dev/kvm`).

## 11. Instructions for the building agent

1. Read this whole document first. Build in milestone order. Never skip acceptance criteria.
2. Start every milestone by writing the public trait/types and tests, then implement.
3. Keep crates small and dependencies one-directional: `types` and `cas` at the bottom, `server` and `cli` at the top. No cycles.
4. Public API changes need a short design note in `docs/adr/NNNN-title.md`.
5. All code must pass `cargo fmt`, `cargo clippy -- -D warnings`, and tests before commit. No `unwrap()` outside tests and `main`.
6. Every `unsafe` block needs a `// SAFETY:` comment and a test.
7. Write docs as you go: each crate gets a README with purpose, invariants, and example.
8. Prefer measured decisions: when the plan says "decide by benchmark," write the benchmark, record results in `docs/decisions/`, then choose.
9. When blocked by an ambiguity, pick the simplest option that preserves the invariants in section 2, document the assumption, and continue.
10. Commit small, one logical change each, with messages that say why.

## 12. Top risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Shell compatibility long tail | Agent-transcript corpus from day one; track pass rate; fall through to Tier 2 on `NeedsVm` rather than faking behavior |
| `deno_core` breaking releases | Facade crate, exact pins, scheduled upgrade windows |
| Isolate escape | Per-tenant process, seccomp, no shared heaps, external review |
| Promotion fidelity | Document that processes don't survive; expose `promote_hint`; fast re-run of failed command |
| Tier 2 cost creep | Aggressive idle sleep, snapshot resume, warm pool sizing by demand |
| WASM tool gaps (git, python) | Native Rust `gix` first; keep VM fallback path always available |
| Snapshot storage growth | CDC dedup, zstd, GC from live roots, retention policies |

## 13. Definition of done (v1)

- All milestones M0-M5 accepted.
- Public API stable and documented; OpenAPI + Rust SDK + TS SDK published.
- Benchmarks meet section 6 targets on reference hardware.
- Security review complete with no open high-severity findings.
- Three working examples (coding agent, data agent, browser agent) and a one-page quickstart that gets a user to a running `exec` in under two minutes.

## 14. Status and open items (2026-10-19)

This section records what the code does today against the plan above. Update it when a milestone changes state.

### Done

- **Isolate tier, shell, and VFS (M1 part):** overlay VFS with commit to a Merkle tree and quotas. Builtin shell with pipes, redirects, and variables. Runs over the VFS in process.
- **Sessions:** create, exec, file read, write, and delete, snapshot, fork, sleep, and wake (ADRs 0004, 0006). Sessions are saved when created and forked, so a restart keeps them (ADR 0014).
- **Durable state (§4.15, §4.8):** `serve --data-dir` keeps keys (as digests), snapshots, and every session with its checkpoint: files and working directory. The environment is kept in memory only, because its values are often secrets. Live sessions come back running after a restart, and sleeping sessions wait for a wake (ADRs 0008, 0010).
- **Traces (§4.12):** commands are recorded per session and served as JSON Lines. `frontal-sandbox replay` re-runs a trace and reports divergences (ADRs 0007, 0009). File writes are not recorded yet, which is the M2 requirement "trace shows every file change".
- **Watch (§4.4):** the VFS delivers change events for a path, with an overflow signal (ADR 0005).
- **API keys and limits (§4.15):** per-key session and snapshot limits, owner-only access, and keys stored as digests.
- **CLI and SDKs (§4.16):** `run`, `shell`, `replay`, `bench`, `serve`, `snapshot`, and `fork`. TypeScript, Python, and Rust clients cover sessions, exec, snapshots, sleep, and wake.
- **Security scanning:** CodeQL uploads results to code scanning. Dependabot alerts are fixed. Secret scanning and push protection are enabled.
- **Releases:** `v0.1.0` and `v0.1.1` publish CLI binaries for five platforms, and an npm wrapper that runs the matching binary (`@frontal-labs/frontal-sandbox`).

- **Content-defined chunks (§4.2, ADR 0017):** blobs above 64 KiB are split with FastCDC. Each distinct chunk is stored once, in a store-wide area, zstd-compressed, and checked against its hash on read. Manifest version 2 is written, and version 1 is still read. The old code stored a chunk once per blob and stored chunks raw. Property tests cover round trips, edit stability, and sweeps (`crates/sandbox-cas/src/disk_store.rs`).
- **Supply chain (§5):** `deny.toml` and `.cargo/audit.toml` exist, and `ci.yml` runs both in the `supply-chain` job. The license policy allows permissive licenses only. Fixed: wasmtime and wasmtime-wasi are now 36.0.19 and rustls 0.23.46 (RUSTSEC-2026-0346 to -0349, -0352, -0363). The GPL-2.0 `ittapi` left the build when wasmtime's `profiling` default was turned off, and the unmaintained `fxhash` went with it. Unused `wizer` and `webpki` were removed. `sqlx` default features are off, which drops the MySQL driver and the `rsa` crate.
- **Fuzzing (§5, §7):** `fuzz/` has five cargo-fuzz targets: `shell_parse`, `shell_run`, `vfs_paths`, `wasi_fs`, and `guest_protocol`. `guest_protocol` decodes the length-prefixed frames and every generated guest message (`sandbox_proto::v1::guest`, `types::ExecRequest`), and checks that each decoded message survives an encode and decode round trip. A 90 s run covered 1,506,992 inputs with no crash. `wasi_fs` drives `path_open`, `path_create_directory`, `path_unlink_file`, and `path_rename` through generated WAT and asserts that no path escapes the sandbox root. Results are under Environment.
- **Streaming exec, per command (§4.15, ADR 0020):** `POST /v1/sessions/{id}/exec` with `?stream=true` returns NDJSON `ExecEvent` lines as each command finishes. `IsolateBackend::exec_streamed` drives it, and the Rust SDK's `Client::exec_stream` reads it. Tests: `crates/sandbox-shell/tests/streaming.rs` (13 tests, including the rule that chunks add up to the final output), `crates/sandbox-server/tests/rest.rs` (4 streaming tests), `sdks/rust/tests/against_server.rs` (2 streaming tests over HTTP). A command that times out ends with an `error` line.
- **Shell conformance (M1 acceptance, ADR 0019):** 219 cases in `crates/sandbox-shell/conformance/cases.json`, recorded from GNU bash 5.3.8 by `record.py`. All 219 match on stdout and exit status (`tests/conformance.rs`). The first run passed 146. The gaps were builtins, parameter operators, `case`, arithmetic, field splitting, and fd redirection. `printf`, `read`, `type`, `grep`, and `tail` are now builtins. `${x:?}` and file descriptors above 2 are refused with a parse error. Feature tests in `tests/features.rs` cover behaviour the corpus does not isolate.
- **Shell robustness (ADR 0018):** recursion through functions, subshells, substitutions, and loops is bounded at 32 levels. A limit aborts the run, and an aborted run exits 1. Glob matching is polynomial. It was exponential in the number of `*`. `shell_run` found both defects. Regression tests: `crates/sandbox-shell/tests/recursion.rs` and `globbing.rs`.
- **Dependency hygiene:** `schemars` was declared in 17 crates and used in none, so it was removed. The OpenAPI spec comes from `utoipa` (ADR 0001).

### Open, and planned in the milestones above
- **Streaming exec: per command is built (ADR 0020). Per byte is open.** Per-byte streaming means changing every builtin and coreutil to write to a sink, not only the interpreter. It needs its own ADR before it is built.
- **Threat model (§5):** `docs/THREAT_MODEL.md` covers assets, actors, trust boundaries, controls cited to code, and residual risks. It states that the worker and VMM are shared-process and have no compute limits, and that no external review has happened.
- **Two guest protocols in the tree:** `proto/sandbox/v1/guest/guest_service.proto` is the contract, with `Ping`, `ReadFile`, `WriteFile`, `ListDir`, and `Sync`, and `Exec` through `types::ExecRequest`. `crates/sandbox-guest/src/protocol.rs` defines a second, ad hoc `ExecRequest`/`FileRequest` set that nothing uses. It should be removed in favour of the proto types. It is not deleted yet, so the change stays reviewable.
- **The guest does not exist.** The `GuestService` schema and the frame decoder are fuzzed, but there is no agent to serve them, so the Tier 2 path is untested end to end.
- **GitHub CI:** the `supply-chain` job and the existing jobs have not been seen passing on GitHub.
- **Lockfile residue:** `rsa` (RUSTSEC-2023-0071, no fixed version) stays in `Cargo.lock` as an optional `sqlx-mysql` entry. Cargo did not prune it when `sqlx`'s features changed. No build compiles it, and `.cargo/audit.toml` ignores the advisory for that reason.
- **Unmaintained advisories, ignored with reasons in `deny.toml`:** `paste` (via `deno_v8`), `proc-macro-error` and `unsafe-libyaml` (via utoipa 4), `rustls-pemfile` (via tonic 0.12), and `number_prefix` (benchmarks). Each waits on an upgrade.
- **Duplicate crate versions:** `cargo deny` reports about 40. They are warnings, not failures.
- **Temporary files from a crashed write** are not removed. The collector ignores them (ADR 0017).

- **VM tier (§4.10, §4.11), Firecracker, promotion, guest agent:** M4.
- **Worker isolation (§4.13, §5), on Linux (ADR 0022):** the worker unshares its network, IPC, UTS, and mount namespaces, installs a seccomp denylist with `NO_NEW_PRIVS`, and runs in a cgroup v2 with memory, pid, and CPU limits. Each layer fails closed. The host needs a delegated cgroup in `SANDBOX_CGROUP_ROOT`, which a systemd unit with `Delegate=yes` provides. The seccomp list is a denylist, so an allowlist is the next step. The cost of the layers on a running command has not been measured. Tests run in the Linux container (`docker/run-linux.sh`). On macOS the isolation is `Unsupported`, and the worker does not start with it. Remaining (§5): the Tier 2 VM and an external review.
- **V8 runtime (§4.5) and WASM tools (§4.6):** `node -e` and `node FILE` run JavaScript in V8 with a 5 s wall limit and a 64 MiB heap cap. `node FILE.ts` strips the types with swc and then runs the result (ADR 0024). The types are not checked. A warm pool of two isolates makes the median `node` run 0.41 ms, down from 5.10 ms, and the tail under back-to-back load is the cold path (ADR 0025). The `wasm` builtin runs WASI preview1 command modules. Not built: the `node:` module shim and the M2 tools (`jq`, `ripgrep`, `sqlite3`, `git`, `python`).
- **Streaming output (§4.15, ADR 0020, ADR 0021):** the server streams NDJSON as each command finishes. `node` and WASI tools also stream each write as it happens, through the runtime and WASI observers. Shell builtins still send at the end of their command, because they build their output in buffers. A tool's stderr is sent when its command finishes. The WebSocket exec endpoint is not built, and NDJSON covers the requirement.
- **WebSocket exec (§4.15):** not built. NDJSON covers the streaming requirement.
- **Exec timeouts (§4.15):** enforced on the isolate tier. A command past `timeout_ms` is stopped at its next await point and returns `TIMEOUT` (HTTP 504). A command that never awaits cannot be interrupted, so the shell's command budget bounds it. The session's `wall_timeout_secs` (300 s by default) caps each command, and the server accepts only the default. It is not a limit on the session's lifetime. The 504 comes at the deadline, but it cannot stop a running V8 script. The runtime's own 5 s limit ends the script, so the V8 thread can keep running for up to 5 s after the 504.
- **Idle sleep (§4.14, ADR 0015):** implemented. `sandbox serve --idle-sleep-secs` (default 300) puts sessions to sleep after that long with no request, and a request wakes them. Metering is per-key counters in memory at `GET /v1/usage` (sessions, commands, command wall time, bytes written). They reset on restart. Usage events to a durable queue, memory-time, and CPU-time metering are open, because CPU time is not measured and memory is not limited.
- **Per-session limits (§4.7, ADR 0014):** `max_file_size`, `max_total_size`, `max_inodes`, and `max_path_depth` are enforced through the VFS, and kept across sleep, wake, restart, and fork. Memory, CPU, process, and open-file limits, and any wall timeout other than the default, are refused with `501` until they are enforced.
- **Garbage collection (§4.2, M5):** implemented as mark and sweep from live roots, with the chunk sweep (ADR 0013). `sandbox serve` runs it every 600 seconds by default (`--gc-interval-secs`), and `POST /v1/gc` runs it on request, at most once a minute. Each command adds checkpoint blobs (ADR 0011). A collection holds every session's gate, so a long command delays it.
- **Per-command checkpoint cost (ADR 0011):** every command commits the visible tree twice to compute its file changes. A checkpoint hashes only the writes since the previous one, but it rereads and rewrites each changed directory's entry list, so the cost grows with directory width. Measured at about 1.5 ms per command with 2000 entries in one directory (debug build). Revisit if directories grow into the tens of thousands of entries.
- **Renames and chmod in traces (ADR 0011):** a rename in a command appears as a delete and a create, and permission changes are not reported, because the committed tree stores neither. The filesystem API has no rename or chmod, so this covers commands only.
- **Durable store (§4.8, ADR 0016, ADR 0023):** log-structured on one node (the default). `serve --features clustered --database-url …` (or `SANDBOX_DATABASE_URL`) keeps the metadata in Postgres as one conditional document with a revision, so two nodes cannot overwrite each other. The blobs stay in `--data-dir`, which must be shared across nodes. The clustered scheduler (placement across nodes and the worker transport) is not built.

### Decisions (2026-10-19)

- **OpenAPI comes from `utoipa`, not `schemars`.** PLAN §4.15 allows either. ADR 0001 chose `utoipa`, so `schemars` is not a dependency.
- **`wizer` is not declared until M2 needs it.** When it returns, it must not enable wasmtime's default features, because `profiling` links GPL code.
- **`sqlx` default features are off.** The clustered scheduler needs Postgres only, and the MySQL driver pulls in `rsa`.
- **The command depth limit is 32** (ADR 0018). It was measured: a debug build overflows a 1 MiB stack at 64 levels and fits 32.
- **`crates/sandbox-server/tests/idle.rs` was timing-dependent.** Its busy loop was `i < 1e9`, which finishes within the 1.5 s timeout on a fast machine. It now runs `while (true) {}`, which the runtime's 5 s limit stops. This file is untracked, so the change sits in uncommitted work.
- **Supply-chain tool versions are pinned in CI:** `cargo-deny` 0.20.2 and `cargo-audit` 0.22.2, the versions the policy was checked with.

### Environment (resolved 2026-10-19)

- **Final gates, 2026-10-19 (after the last code change):** `cargo fmt --all -- --check` passed. `cargo clippy --workspace --all-targets -- -D warnings` passed. `cargo test --workspace --no-fail-fast` passed: 59 suites, 308 tests, 0 failed, 1 ignored (the ignore predates this work). `cargo bench --workspace --no-run` compiled. `cargo deny check` and `cargo audit --deny warnings` passed. These ran on macOS. The GitHub run is not observed.
- **Fuzzing, nightly, 2026-10-19:** `shell_parse` ran 782,404 inputs in 91 s with no crash. `vfs_paths` ran 3,830,897 inputs in 91 s with no crash, and the "no path climbs above root" assertion held. `shell_run` found a stack overflow (fixed, ADR 0018) and then a 60-second glob hang (fixed). After both fixes, a 120-second run covered 498,506 inputs with no crash or timeout, and the saved hang input runs in 3 ms.
- **Verified 2026-10-20 (after the shell and WASI fs work):** `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed. `cargo test --workspace --no-fail-fast` passed. The shell conformance corpus passed 219 of 219 against GNU bash 5.3.8. `cargo deny check` and `cargo audit --deny warnings` passed on the current lockfile, including `regex` as a direct dependency of `sandbox-shell`. `wasi_fs` ran 3,786 inputs in 91 s with no crash or assertion failure.
- **Verified 2026-10-20 (streaming exec and guest protocol fuzzing):** `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed. `cargo test --workspace --no-fail-fast`: 367 passed, 0 failed, 1 ignored (the ignore predates this work). `cargo bench --workspace --no-run` compiled. `cargo deny check` and `cargo audit --deny warnings` passed. The fuzz crate builds all five targets. `guest_protocol` ran 1,506,992 inputs in 91 s with no failure. The OpenAPI spec was regenerated with `gen_openapi`, and the drift tests pass. Not verified: the GitHub CI run, and any Linux-only path.
- **Verified 2026-10-20 (worker isolation, TypeScript, warm isolates, Postgres store, per-write streaming):** Linux (`docker/run-linux.sh`, Docker Desktop, aarch64 kernel 6.10): `cargo test --workspace --no-fail-fast` passed, 391 tests, 0 failed, 1 ignored, with `CARGO_BUILD_JOBS=3` and no test debug info (the link step was killed by memory at full parallelism). Includes six worker isolation tests on a real kernel and two seccomp fork tests. `--features clustered` tests: 2 passed against Postgres 18.6. macOS: `cargo test --workspace` passed, 383 tests, 0 failed, 1 ignored. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` (Linux and macOS), `cargo clippy --features clustered` passed. `cargo deny check` and `cargo audit --deny warnings` passed. Cold start (release, Linux): `session_create` p50 833 ns; `node_script` p50 0.41 ms warm, 5.10 ms cold, with a tail under back-to-back load. Not verified: the GitHub CI run; the Bazel targets (not updated for the new dependencies, and Bazel was not run); the fuzz crate after this pass.

- **Disk full:** the earlier `ENOSPC` blocker is cleared. The system volume has about 82 GiB free and the project volume about 337 GiB, and writes succeed. On 2026-10-19 `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all passed after the file-change trace change (ADR 0011).
- **Docker:** `docker` cannot reach its daemon (the socket at `~/.docker/run/docker.sock` refuses connections), so nothing was removed. Starting Docker Desktop and confirming the scope of any cleanup is pending the operator.

### Blocked on the operator

- **Package visibility:** the GHCR SDK packages (`sandbox-sdk-ts`, `-py`, `-rs`) and the npm packages (`@frontal-labs/frontal-sandbox` and its platform packages) are published as `internal`. GitHub provides no API or GraphQL mutation for changing organization package visibility, so an organization owner must change it in the web UI before anonymous pulls and installs work.
