# Sandbox Guest

In-VM guest agent for Tier 2 microVMs. Planned as a static musl binary.

**Status: scaffolded.** The crate defines a Unix-socket listener using the protobuf types from `sandbox-proto`. The connection handler does nothing, so no requests are served yet. There is no vsock transport and no binary target.

## Public API

Re-exported from `src/lib.rs`:

- `GuestAgent` (`src/agent.rs`)
  - `new(socket_path: &str)`: binds a `tokio::net::UnixListener`.
  - `run(&self)`: accepts connections in a loop and spawns a task per connection.
- `guest` (`src/lib.rs`): re-exports the protocol types from `sandbox-proto::v1::guest`
  - `GuestService` gRPC service: `Ping`, `Exec`, `ReadFile`, `WriteFile`, `ListDir`, `Sync`
  - Message types: `PingResponse`, `ReadFileRequest`, `ReadFileResponse`, `WriteFileRequest`, `ListDirRequest`, `ListDirResponse`, `DirEntry`, `SyncRequest`, `SyncResponse`

Cargo feature `vsock` enables an optional `vsock` dependency. Nothing in the crate uses it yet.

## Differences from the planned protocol

The PLAN describes length-prefixed frames over vsock with `Exec`, `Signal`, `ReadFile`, `WriteFile`, `ListDir`, `Sync`, and `Ping`, plus streamed stdout/stderr. The current code uses the protobuf service definition which matches the PLAN's messages, but has no framing, no streaming, no signal handling, and listens on a Unix socket instead of vsock.

## Invariants

- None enforced yet. The message types are data only.

## Security assumptions (planned)

- The VM holds no host credentials.
- Egress is controlled by host nftables, mirroring `sandbox-policy`.
- Host communication is over vsock only, with no network exposure. The current code listens on a Unix socket path instead, so this does not hold yet.
- The guest must authenticate host requests before serving them. No authentication exists yet.

## Spec

See [plans/PLAN.md](../../plans/PLAN.md) section 4.10. Build and image integration (`tools/`, musl target) is not in this crate yet.

## License

Apache-2.0
