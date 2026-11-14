# External Security Review

**Status: not yet performed.**

The project specification (section 5) lists an external security review of the
worker and VMM as a prerequisite for any public launch. As of this writing no
external review has been commissioned, and this document exists to record that
honest status rather than imply one.

## Why this document exists

`docs/SECURITY_MODEL.md` describes the intended defense-in-depth layers and
marks each as Implemented, Partial, or Planned. That document is a self-assessment
by the project authors. An external review is a separate, independent
assessment: it tests the actual code and deployment against the threat model,
attempts real escapes, and produces a findings report that the authors must
then disposition (fix, mitigate, or accept).

Until that review has been done, the production claim in the project
specification should be read as **pre-alpha / not launch-ready**, consistent
with the status line at the top of `docs/SECURITY_MODEL.md`.

## Planned scope (for when the review is commissioned)

The review should cover at minimum:

1. **Worker isolation** (`crates/sandbox-worker/src/isolation.rs`)
   - Seccomp denylist completeness: does it block every syscall that could
     escape the namespaces?
   - Namespace configuration: are the network, IPC, UTS, and mount namespaces
     actually independent of the host?
   - Cgroup v2 limits: are `memory.max`, `pids.max`, and `cpu.max` enforced
     and does a cgroup exhaustion correctly fail-closed?
   - `NO_NEW_PRIVS`: confirmed set before any `execve`?

2. **VMM / Tier 2** (`crates/sandbox-vmm/src/qemu.rs`)
   - QEMU configuration: are the network, USB, and virtio devices configured
     to minimize the guest-to-host attack surface?
   - KVM vs TCG: is the security boundary clearly documented as KVM-only?
   - Guest agent protocol: is the wire protocol validated against malformed
     or malicious frames? (The fuzz target in `fuzz/` covers this, but an
     external review should attempt real exploits.)

3. **V8 / WASM runtimes**
   - Is the V8 heap limit actually enforced? What happens on a near-limit
     OOM?
   - Is the wasmtime fuel limit and linear-memory cap correctly enforced?
   - Can a script escape its isolate and access host memory?

4. **Network egress**
   - Is `NetworkPolicy::default()` (default-deny) actually enforced at the
     connection level, or is it only a type-level default?
   - Is DNS pinning implemented (to prevent DNS-rebinding)?
   - Are private/metadata IP ranges (`169.254.169.254`, `100.64.0.0/10`,
     `0.0.0.0/8`, IPv4-mapped IPv6) blocked?

5. **Authentication and authorization**
   - API key validation: is the BLAKE3 comparison constant-time?
   - Key revocation: does a revoked key immediately stop working?
   - Is there any path that bypasses `require_api_key`?

6. **Supply chain**
   - Are V8, wasmtime, and QEMU version-pinned and CVE-tracked?
   - Does `cargo deny` and `cargo audit` cover all dependencies including
     transitive ones?

## Findings

_No findings recorded. No review has been performed._

## Disposition

_Not applicable. No findings to disposition._

## How to record the review

When the external review is completed, update this document with:

- **Reviewer:** name of the firm or individual, and their security specialism
- **Date:** start and end dates of the engagement
- **Scope:** which of the sections above were in scope (and any exclusions)
- **Findings:** for each finding, severity (Critical / High / Medium / Low /
  Informational), description, CVSS score if provided, and the code path
- **Disposition:** for each finding, the decision (fix / mitigate / accept),
  the linked PR or commit that addresses it, and the date it was closed
- **Re-review:** if any findings were accepted rather than fixed, note when
  they should be re-examined

A copy of the full findings report (if the reviewer permits sharing) should be
linked from this document or stored in a private location referenced here.
