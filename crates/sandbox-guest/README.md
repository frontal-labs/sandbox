# Sandbox Guest

In-VM guest agent for Tier 2 microVMs. Runs as the guest's init (or a child of a tiny
init), serving the host over the gRPC service in `proto/sandbox/v1/guest/`.

**Status: working.** The agent listens on TCP (the host forwards a loopback port to it)
or on a Unix socket, and implements the full `GuestService`: `Ping`, `Exec`, `ReadFile`,
`WriteFile`, `ListDir`, `MkDir`, `Rename`, `Remove`, `Stat`, `Sync`, `Signal`. `Exec`
runs a command via `sandbox-shell` over `sandbox-vfs` and streams stdout/stderr. The
binary target `sandbox_guest_agent` is booted as init in the emulated test guest
(`docker/emulated-vm/`).

## Public API

Re-exported from `src/lib.rs`:

- `GuestAgent` (`src/agent.rs`)
  - `new_tcp(addr: SocketAddr)`: binds a `tokio::net::TcpListener`.
  - `new_unix(socket_path: &str)`: binds a `tokio::net::UnixListener`.
  - `run(&self)`: accepts connections in a loop and spawns a task per connection,
    serving the `GuestService` gRPC on each.
- `guest` (`src/lib.rs`): re-exports the protocol types from `sandbox-proto::v1::guest`
  - `GuestService` gRPC service: `Ping`, `Exec`, `ReadFile`, `WriteFile`, `ListDir`,
    `MkDir`, `Rename`, `Remove`, `Stat`, `Sync`, `Signal`
  - Message types: `PingResponse`, `ReadFileRequest`, `ReadFileResponse`,
    `WriteFileRequest`, `WriteFileResponse`, `ListDirRequest`, `ListDirResponse`,
    `DirEntry`, `MkDirRequest`, `MkDirResponse`, `RenameRequest`, `RenameResponse`,
    `RemoveRequest`, `RemoveResponse`, `StatRequest`, `StatResponse`,
    `SyncRequest`, `SyncResponse`, `ExecRequest`, `ExecResponse`, `SignalRequest`,
    `SignalResponse`

## Invariants

- The agent is the guest's only long-running process besides `bash` (and, in the desktop
  image, Xorg/Xfce). It reaps children.
- `Exec` runs through `sandbox-shell`, which resolves builtins against the VFS.
- `Sync` pushes the guest VFS state back to the host VFS.

## Security assumptions

- The VM holds no host credentials.
- Egress is controlled by whatever network interface the VMM exposes to the guest; the
  current driver forwards only the agent port on loopback.
- Host communication is over the forwarded loopback port (or a Unix socket on vsock-backed
  guests), with no other network exposure.
- The guest does not authenticate host requests; the host is trusted (it owns the VMM).

## Spec

See the project specification section 4.10. Build and image integration (`tools/`, musl
target, base and browser images) is not in this crate yet; see `plans/IMAGE.md`.

## License

Apache-2.0
