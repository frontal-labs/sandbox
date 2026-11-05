# 0027: Guest protocol v1

## Status
Accepted

## Date
2026-10-25

## Context
plans/IMAGE.md §4.3 extends the guest protocol (PLAN.md §4.10) for snapshot/clone hygiene and agent-native operations. The host must always dial the guest (guest listens on vsock). After snapshot load, `Reseed` is required before any other frame. The repo already has protobuf definitions in `proto/sandbox/v1/guest/guest_service.proto` and `proto/sandbox/v1/types/exec.proto`.

## Decision
- **Transport:** vsock, guest listens on `SANDBOX_GUEST_PORT` (constant in `sandbox-types`). Length-prefixed protobuf frames generated from `proto/` with `tonic`/`prost`.
- **Connection flow:** Guest sends `Hello { protocol_version, image_id }` on every connection. Host sends `Reseed { entropy[64], machine_id, boot_id, hostname, realtime_ns, xauth_cookie? }` after every snapshot load (required). Until `Reseed` succeeds on a post-reset connection, only `Hello` and `Ping` are allowed; others return `NOT_READY`.
- **v1 frames (additions):** `Hello`, `Reseed`, `PrepareSnapshot {}`, `EnvInfo { json }`, `ForkRequest { n, argv }`, `UndoRequest { steps }`, `DesktopStart { width, height }`. Existing: `Exec`, `Signal`, `ReadFile`, `WriteFile`, `ListDir`, `Sync`, `Ping`.
- **Snapshot hygiene:** Before snapshot, host calls `PrepareSnapshot` and guest quiesces (no credentials in VM, close outbound connections owned by guest, sync). After load, host does `Reseed` (RNDADDENTROPY + RNDRESEEDCRNG, new machine-id, new boot_id bind-mount, hostname, clock, X auth cookie) and guest replies with new values for verification.
- **Agent-native requests:** `ForkRequest` and `UndoRequest` originate from guest via the `sandbox` command and are handled by host with policy/quotas.
- **Fuzzing:** Frame decoder must be fuzzed with `cargo-fuzz` per M4 requirements.

## Consequences
### Positive
- Deterministic clone uniqueness and resilience across snapshot/resume.
- Clear handshake prevents acting on unstable post-snapshot state.
- Extensible protocol for desktop and agent-native operations.

### Negative
- Requires strict ordering and state tracking on both sides.
- Adds complexity to connection handling and testing.

### Risks
- Snapshot timing issues if quiescing is incomplete.
- Backward compatibility if protocol versions diverge.

## Alternatives Considered
- Allowing operations before reseed (rejected: race-prone).
- Encoding reseed in environment only (rejected: not sufficient for kernel CSPRNG/IDs).
