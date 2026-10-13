# sandbox-guest

In-VM guest agent for Tier 2 microVMs. Planned as a static musl binary.

**Status: scaffolded.** The crate defines protocol message types and a Unix-socket listener. The connection handler does nothing, so no requests are served yet. There is no vsock transport and no binary target.

## Public API

Re-exported from `src/lib.rs`:

- `GuestAgent` (`src/agent.rs`)
  - `new(socket_path: &str)`: binds a `tokio::net::UnixListener`.
  - `run(&self)`: accepts connections in a loop and spawns a task per connection.
- `protocol` (`src/protocol.rs`), prost message types:
  - `ExecRequest { argv, env, cwd, stdin }` and `ExecResponse { exit_code, stdout, stderr }`
  - `FileRequest { path, operation, data }` and `FileResponse { data, error }`
  - `FileOperation { Read, Write, List, Stat }`
  - `encode_message` and `decode_message` helpers

Cargo feature `vsock` enables an optional `vsock` dependency. Nothing in the crate uses it yet.

## Differences from the planned protocol

The PLAN describes length-prefixed frames over vsock with `Exec`, `Signal`, `ReadFile`, `WriteFile`, `ListDir`, `Sync`, and `Ping`, plus streamed stdout/stderr. The code has none of the streaming, signal, sync, or ping messages, and no framing.

## Invariants

- None enforced yet. The message types are data only.

## Security assumptions (planned)

- The VM holds no host credentials.
- Egress is controlled by host nftables, mirroring `sandbox-policy`.
- Host communication is over vsock only, with no network exposure. The current code listens on a Unix socket path instead, so this does not hold yet.
- The guest must authenticate host requests before serving them. No authentication exists yet.

## Spec

See [PLAN.md](../../PLAN.md) section 4.10. Build and image integration (`tools/`, musl target) is not in this crate yet.
