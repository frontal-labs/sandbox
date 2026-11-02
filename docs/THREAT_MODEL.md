# Threat Model

## 1. Assets

### 1.1 Primary Assets
- **Session data**: Working directory, environment variables (often contain secrets), shell state, files created during execution
- **User code**: Scripts, binaries, WASM modules executed by agents
- **Credentials**: API keys, tokens injected via network policy secrets
- **Trace logs**: Complete execution history including command output, file changes, network requests

### 1.2 Infrastructure Assets
- **VFS/CAS storage**: Content-addressed blob store with session snapshots
- **Scheduler metadata**: Session state, tenant assignments, resource usage
- **Worker processes**: One per tenant, running with isolation
- **Firecracker VMs**: Tier 2 microVMs (when promoted)

### 1.3 Derived Assets
- **Generated artifacts**: Build outputs, test results, reports
- **Network traffic**: Egress to allowed hosts, responses

## 2. Actors

### 2.1 Trusted
- **Frontal sandbox operators**: Deploy and configure the system
- **API key holders**: Tenants with valid credentials
- **System services**: Scheduler, workers, storage

### 2.2 Semi-Trusted
- **AI agents**: Execute arbitrary code within sessions, but limited by capabilities
- **User-provided code**: Scripts, containers, WASM modules

### 2.3 Untrusted
- **External networks**: Internet, third-party APIs
- **Malicious agents**: Attempting to escape, exfiltrate data, or exhaust resources
- **Compromised dependencies**: Supply chain attacks

## 3. Trust Boundaries

### 3.1 Tier 1 (Isolate) - In-Process Isolation
```
┌─────────────────────────────────────────────────────────────┐
│                    Worker Process (per tenant)              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │   Shell     │  │  V8 Isolate │  │  WASM Store │         │
│  │  (parser,   │  │  (JS/TS)    │  │  (wasmtime) │         │
│  │  interpreter)│  │             │  │             │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
│         │                │                │                 │
│         └────────────────┼────────────────┘                 │
│                          ▼                                  │
│                   ┌─────────────┐                           │
│                   │    VFS      │  (copy-on-write,          │
│                   │  (overlay)  │   content-addressed)      │
│                   └─────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

**Trust Assumptions:**
- V8 isolates and wasmtime stores are NOT security boundaries on their own (PLAN §5)
- The worker process IS the security boundary for Tier 1
- Seccomp, cgroups, namespaces provide the actual isolation

### 3.2 Tier 2 (VM) - Hardware Boundary
```
┌─────────────────────────────────────────────────────────────┐
│                      Host OS                                │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Firecracker VMM                        │   │
│  │  ┌─────────────────────────────────────────────┐   │   │
│  │  │           MicroVM (per session)             │   │   │
│  │  │  ┌─────────────────────────────────────┐   │   │   │
│  │  │  │      Guest Agent (vsock)           │   │   │   │
│  │  │  │  ┌────────┐  ┌────────┐  ┌───────┐  │   │   │   │
│  │  │  │  │ Linux  │  │  VFS   │  │ User  │  │   │   │   │
│  │  │  │  │ Kernel │  │ Export │  │ Code  │  │   │   │   │
│  │  │  │  └────────┘  └────────┘  └───────┘  │   │   │   │
│  │  │  └─────────────────────────────────────┘   │   │   │
│  │  └─────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Trust Assumptions:**
- Firecracker + jailer provide hardware-enforced boundary
- Guest agent communicates only over vsock
- VM network egress controlled by host nftables mirroring sandbox-policy

### 3.3 Cross-Cutting Boundaries
- **Scheduler ↔ Worker**: gRPC over Unix socket (same host)
- **Server ↔ Scheduler**: In-process (single node) or gRPC (clustered)
- **Client ↔ Server**: HTTPS with API key auth
- **VFS/CAS**: Shared storage, no tenant isolation at rest

## 4. Attack Vectors and Mitigations

### 4.1 Code Execution (Tier 1)
| Vector | Mitigation | Status |
|--------|-----------|--------|
| V8 escape | Per-tenant process + seccomp + cgroups + namespaces | ✅ Implemented |
| WASM escape | wasmtime sandboxing + epoch interruption + store limits | ✅ Implemented |
| Shell injection | Parser rejects dangerous constructs, bounded recursion | ✅ Implemented |
| Host FS access | All FS ops go through VFS (no ambient authority) | ✅ Implemented |
| Network access | Default-deny, allowlist by host pattern, pinned DNS | ✅ Implemented |
| Resource exhaustion | Heap limits, CPU watchdog, fuel, quotas, **CPU/process/open-file limits** | ✅ Implemented |

### 4.2 Code Execution (Tier 2)
| Vector | Mitigation | Status |
|--------|-----------|--------|
| VM escape | Firecracker + jailer (chroot, cgroups, netns, seccomp, dropped caps) | ⚠️ **Driver and jailer implemented, not production-ready** |
| Guest agent compromise | Protocol is framed, all commands capability-checked | ⚠️ **Agent implemented, not production-ready** |
| VM-to-VM | Separate tap devices, nftables isolation | ❌ Not implemented |
| Host kernel attack | Minimal kernel, no host mounts in VM | ❌ Not implemented |

### 4.3 Data Exfiltration
| Vector | Mitigation | Status |
|--------|-----------|--------|
| Secret leakage in env | Env values never stored in snapshots (ADR 0010) | ✅ Implemented |
| Network exfiltration | DNS pinning, private range blocks, size/time limits | ✅ Implemented |
| Trace log access | Per-session, owner-only access | ✅ Implemented |
| Snapshot access | Per-key, owner-only | ✅ Implemented |
| VFS escape | Path normalization, depth cap, symlink resolution limit | ✅ Implemented |

### 4.4 Resource Exhaustion
| Vector | Mitigation | Status |
|--------|-----------|--------|
| Memory (V8) | `max_old_space_size`, near-heap-limit callback | ✅ Implemented |
| Memory (WASM) | `StoreLimits`, pooling allocator | ✅ Implemented |
| CPU | Epoch interruption (WASM), watchdog thread (V8) | ✅ Implemented |
| Disk (VFS) | Quotas: total bytes, inodes, max file size, path depth | ✅ Implemented |
| Disk (CAS) | GC mark/sweep from live roots, zstd compression, CDC dedup | ✅ Implemented |
| PIDs | cgroup v2 `pids.max` | ✅ Implemented |
| Open files | cgroup v2 + VFS handle tracking | ✅ Partial |
| Wall time | Per-command timeout, session default 300s | ✅ Implemented |

### 4.5 Supply Chain
| Vector | Mitigation | Status |
|--------|-----------|--------|
| Malicious crate | cargo-deny (licenses, advisories, bans, sources) | ✅ Implemented |
| Unpinned deps | Exact version pins in Cargo.toml, lockfile committed | ✅ Implemented |
| V8/wasmtime CVEs | Automated tracking, scheduled upgrade windows | ✅ Process only |
| Build compromise | Reproducible builds (planned), sigstore (planned) | ❌ Planned |

### 4.6 Lateral Movement
| Vector | Mitigation | Status |
|--------|-----------|--------|
| Tenant A → Tenant B | Separate worker process per tenant, no shared heaps | ✅ Implemented |
| Session → Session | Copy-on-write VFS, independent overlays | ✅ Implemented |
| Worker → Host | `NO_NEW_PRIVS`, seccomp denylist, cgroups, namespaces | ⚠️ Seccomp is denylist (allowlist planned) |
| VM → Host | Firecracker device model, no host devices passed | ❌ Not implemented |

## 5. Controls by Layer (Outer to Inner)

### 5.1 Hardware (Tier 2 Only)
- **Firecracker microVM**: KVM-based virtualization
- **Jailer**: chroot, cgroups v2, network namespace, seccomp, dropped capabilities
- **Block device**: Read-only base image + per-session writable overlay
- **Network**: tap device per VM, nftables egress rules

### 5.2 OS Process (Per Tenant)
- **`prctl(PR_SET_NO_NEW_PRIVS)`**: Prevent privilege escalation
- **Seccomp-bpf**: Syscall filter (currently denylist, allowlist planned)
- **cgroup v2**: `memory.max`, `pids.max`, `cpu.max`, `io.max`
- **Namespaces**: `CLONE_NEWNET`, `CLONE_NEWIPC`, `CLONE_NEWUTS`, `CLONE_NEWNS`
- **No host mounts**: VFS is the only filesystem
- **Read-only binary**: Worker binary immutable

### 5.3 Language Runtime
- **V8**: Heap limit, CPU watchdog, wall timeout, snapshot pooling
- **wasmtime**: Pooling allocator, epoch interruption, `StoreLimits`, fuel

### 5.4 Capability System
- **Capabilities**: `fs:read:<prefix>`, `fs:write:<prefix>`, `net:http:<host>`, `exec:<tool>`, `env:<name>`, `clock`
- **Policy checks**: Every host operation checks capability + resource limits
- **Deny events**: Every denied operation emits trace event
- **Secrets injection**: Session config holds secrets; egress proxy attaches to matching hosts

## 6. Residual Risks (Open)

### 6.1 High Severity
1. **Seccomp is denylist, not allowlist** - **Allowlist implemented, not yet default**
2. **No Firecracker integration** - **Driver and jailer implemented, not production-ready**
3. **No guest agent** - **Agent implemented, not production-ready**
4. **No promotion logic** - Tier 1 → Tier 2 handoff not built
5. **No external security review** - Worker and VMM not reviewed

### 6.2 Medium Severity
6. **Memory/CPU limits not fully enforced** - **V8 near-heap-limit works, cgroup memory tied; CPU, process, open-file limits now enforced in isolate backend**
7. **Shared VFS/CAS storage** - No encryption at rest, no tenant isolation at storage layer
8. **Single-node scheduler** - **Clustered scheduler with Postgres implemented, not production-ready**
9. **DNS pinning best-effort** - Resolves at startup, not per-request
10. **No audit logging** - Only trace events; no tamper-evident audit trail

### 6.3 Low Severity
11. **Temporary files from crashed writes** - **Cleaned up on DiskStore::open (ADR 0017)**
12. **Lockfile residue** - **`rsa` crate (RUSTSEC-2023-0071) removed via cargo update**
13. **Duplicate crate versions** - **Reduced via cargo update**
14. **No per-request DNS resolution** - Vulnerable to rebinding if TTL > 0

## 7. Testing and Verification

### 7.1 Implemented
- **Fuzzing** (cargo-fuzz): 5 targets (shell_parse, shell_run, vfs_paths, wasi_fs, guest_protocol)
- **Conformance**: 219 shell cases match GNU bash 5.3.8
- **Property tests**: VFS, CAS round-trips
- **Unit tests**: 300+ tests across crates
- **Integration tests**: Server + worker + SDK in-process
- **Escape tests**: Attempts to read host files, reach metadata IP, exhaust resources

### 7.2 Missing
- **Tier 2 fuzzing**: Guest protocol fuzzed but no agent to serve
- **Firecracker escape tests**: Not possible without implementation
- **Chaos testing**: Kill workers mid-exec, corrupt blobs, fill disk
- **Determinism tests**: Replay with recorded network responses

## 8. Required Before Public Launch (per PLAN §5)

- [ ] Threat model doc complete ✅ (this document)
- [ ] External security review of worker and VMM ❌
- [ ] Fuzz targets for all parsers and protocol decoders ⚠️ Partial
- [ ] Escape test suite passes ⚠️ Tier 1 only
- [ ] Never share V8 heap or wasmtime store across tenants ✅
- [ ] Pin and track V8/wasmtime/Firecracker CVEs ✅ Process
- [ ] cargo-deny and cargo-audit in CI ✅
- [ ] `unsafe` only in named modules with comments ⚠️ Check needed

## 9. Control Coverage Summary

| Layer | Tier 1 | Tier 2 |
|-------|--------|--------|
| Hardware | N/A | ⚠️ **Driver implemented** |
| OS Process | ✅ | ⚠️ **Jailer implemented** |
| Language Runtime | ✅ | ✅ (same) |
| Capability System | ✅ | ⚠️ **Via guest agent** |
| Network | ✅ | ❌ |
| Storage | ✅ | ✅ (shared VFS) |

## 10. Recommendations

### Immediate (M3)
1. Convert seccomp from denylist to allowlist
2. Implement worker isolation tests on Linux CI
3. Add per-request DNS resolution for egress

### Short-term (M4)
4. Implement Firecracker driver and jailer
5. Build sandbox-guest static musl binary
6. Implement promotion logic with VFS export_ext4
7. Build base and browser VM images

### Medium-term (M5)
8. External security review
9. Clustered scheduler with Postgres
10. Object-store CAS with encryption
11. Tamper-evident audit logging
12. Reproducible builds with sigstore

---
*Document version: 2.0*
*Generated: 2026-11-02*
*Based on: PLAN.md, ADRs, code review*