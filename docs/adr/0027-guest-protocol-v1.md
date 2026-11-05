# 0027: Guest protocol v1

## Status
Proposed

## Date
2026-10-25

## Context
plans/IMAGE.md §4.3 defines the host-to-guest protocol for Tier 2. Its rules are the hard part. The guest listens on vsock and the host dials, because Firecracker closes open vsock connections on snapshot while listening sockets survive. The guest speaks first with `Hello`. After a snapshot load the host sends `Reseed` before anything else, and the guest rejects every other frame until `Reseed` succeeds. IMAGE §4.6 uses the same handshake for snapshot hygiene. Output has to stream while a command runs.

The repository does not implement this yet, and the contract it has does not fit:

- `proto/sandbox/v1/guest/guest_service.proto` defines a unary gRPC `service GuestService`. A unary service cannot express a guest-first `Hello`, a per-connection gate, or output streamed from the guest without a request. ADR 0001 says the guest contract is protobuf, and it cites PLAN §4.10 for "protobuf frames to the guest agent". So the service definition is out of line with the accepted ADR, and this ADR replaces it.
- `crates/sandbox-guest/src/protocol.rs` holds a second, ad hoc `ExecRequest` and `ExecResponse` that nothing uses. PLAN §14 already calls for removing it in favour of the proto types.
- `crates/sandbox-proto/src/framing.rs` already implements the length prefix: a 4-byte big-endian length, capped at 16 MiB and checked before any allocation.

The guest runs user code, so it is untrusted. Every frame the host reads from it is hostile input, including the requests that ask the host for work.

## Decision
- **Transport.** vsock. The guest listens on `SANDBOX_GUEST_PORT`, a constant in `sandbox-types`. The host dials after every resume and after every cold boot. One connection is one generation of the protocol state.
- **Framing.** Each frame is the 4-byte big-endian length from `framing.rs`, followed by one protobuf `Frame`. The 16 MiB cap applies to every frame.
- **Envelope.** `Frame` carries a `uint64 id` and a `oneof body`. A request has a non-zero `id`, and its reply echoes that `id`. Frames the guest sends without a request, such as `Hello` and output chunks, use `id = 0`. Errors are a reply carrying an `Error` body.
- **Message set v1.**

  | Direction | Frame | Purpose |
  | --- | --- | --- |
  | guest → host | `Hello { protocol_version, image_id }` | First frame on every connection |
  | guest → host | `ExecOutput { stream, data }` | One chunk of a running command's stdout or stderr |
  | guest → host | `ExecExit { exit_code, wall_ms, cpu_ms }` | The command's reply, after its last output chunk |
  | guest → host | `ForkRequest { count, argv }` | Asks the host to fork the session (`sandbox fork`) |
  | guest → host | `UndoRequest { steps }` | Asks the host to roll back (`sandbox undo`) |
  | host → guest | `Reseed { entropy[64], machine_id, boot_id, hostname, realtime_ns, xauth_cookie? }` | Required before any other frame on a connection (IMAGE §4.6) |
  | host → guest | `PrepareSnapshot {}` | Quiesces the guest; replies `Ready` when a snapshot is safe |
  | host → guest | `EnvInfo { json }` | Writes `/etc/agent/env.json` |
  | host → guest | `Exec { argv, env, cwd, stdin, timeout_ms }` | Runs one command as user `agent` |
  | host → guest | `Signal { pid, signal }` | Sends a signal to a command's process group |
  | host → guest | `ReadFile`, `WriteFile`, `ListDir`, `Sync` | VFS operations; `Sync` replaces `SyncRequest` in `guest_service.proto` |
  | host → guest | `Ping {}` | Liveness; allowed before `Reseed` |
  | host → guest | `DesktopStart { width, height }` | Starts Xorg and Xfce; replies when ready (I4) |

  Each reply is a frame with the request's `id`. Its body is the matching `…Reply` message, or `Error`.
- **Gate.** On every connection, the guest accepts only `Hello` and `Ping` until a `Reseed` has succeeded on that connection. Any other frame gets `Error { code: UNAVAILABLE, reason: "NOT_READY" }`, and the frame is not executed. `Reseed` replies `ReseedAck` with the values the guest now holds, so the host can check them. IMAGE §4.6 gives the steps.
- **Gate scope (decision).** IMAGE requires `Reseed` after every snapshot load. This ADR requires it on every connection, including the first connection after a cold boot. A reconnect without a snapshot therefore costs one round trip and resets the machine ID, the boot ID, and the clock. The stricter rule means the protocol never depends on the guest knowing whether a snapshot was loaded.
- **Versioning.** `Hello.protocol_version` is `1`. A host that does not speak the guest's version closes the connection. The guest closes it when the host's `Reseed` is missing or the version is unknown. A new variant is added with a new `oneof` number and is never reused. A frame the guest does not know gets `Error { code: UNSUPPORTED }` and the connection stays open.
- **Host policy on guest requests.** The host treats `ForkRequest` and `UndoRequest` as requests, not commands. It checks `count`, the per-key quota, and the session's live-child count before it acts (ADR 0030). It also checks `steps` against the retained checkpoints. The guest cannot make the host act beyond those checks.
- **Sizes.** `ReadFile` and `WriteFile` carry their data in one frame, so the cap is 16 MiB. A larger file gets `Error { code: RESOURCE_EXHAUSTED }`. Chunked file transfer is a v2 concern.
- **Retired.** The `GuestService` gRPC service and `protocol.rs` are removed in I1, when the frame codec and the guest agent are written. Until then the existing `guest_service.proto` stays as the contract and is not used.
- **Fuzzing.** The `guest_protocol` fuzz target decodes `Frame`. The current target decodes the `GuestService` messages, so it changes in I1. IMAGE §5 requires fuzzing the decoder before I2 is accepted.
- **Error codes.** `NOT_READY` is not a `sandbox_types::ErrorCode` today. This ADR uses `UNAVAILABLE` with the `reason` field until a later ADR adds a dedicated code.

## Consequences
### Positive
- The handshake is in the protocol, not in the caller's discipline. A missing `Reseed` cannot run a command on a cloned VM.
- Output streams as `ExecOutput` frames. The host can reuse the NDJSON chunking rule from ADR 0020.
- One framing module (`framing.rs`) serves the worker and the guest.

### Negative
- Two sides keep state per connection, and the ordering rules need tests in both.
- Requests the guest makes (`ForkRequest`, `UndoRequest`) are new host-side surface. Each needs its own checks and its own tests.
- A reconnect costs a `Reseed`, which is stricter than IMAGE requires.

### Risks
- A guest-first `Hello` that is never sent, or a host that dials before the guest listens, leaves the connection waiting. The host needs a connect timeout.
- An unbounded `id` space or unanswered requests can leak state. The host caps outstanding requests per connection in I2.
- Firecracker behavior is cited from IMAGE §13, which says it was checked on 2026-10-25 against the public docs. It has not been re-verified against the pinned release. The rules here depend on the vsock reset and listen-socket behavior, so they need that check before I2.

## Alternatives Considered
- **Keep the unary gRPC `GuestService`.** Rejected. A unary service cannot send `Hello` first, and it cannot hold the gate per connection.
- **Bidirectional gRPC over vsock with tonic.** Rejected for v1. It adds HTTP/2 framing to a static musl binary, and the gate would still be ours to build.
- **MessagePack frames.** PLAN §4.10 allows them. Rejected because ADR 0001 chooses protobuf for internal contracts, and the rest of the repository uses it.
- **Allow VFS and exec before `Reseed`.** Rejected. IMAGE §4.6 says a clone that runs before reseeding can reuse PRNG state, which is the race the rule exists to close.
- **Reseed only after a snapshot load, as IMAGE states.** Kept as a possible relaxation. Rejected for v1 because the guest cannot tell a snapshot load from a reconnect.

## References
- plans/IMAGE.md §2 (invariants 3 and 5), §4.3, §4.6, §5, §7, §13
- plans/PLAN.md §4.10, §14
- [ADR 0001](0001-protobuf-and-openapi-contracts.md): protobuf for internal contracts
- [ADR 0020](0020-streaming-exec.md): NDJSON chunking rule for streamed output
- [ADR 0026](0026-package-layers-and-lockfile.md): the lockfile the guest reads
- The host side of `ForkRequest` and `UndoRequest` is planned as a separate ADR, written with the Tier 1 checkpoint and fork work in I0.
