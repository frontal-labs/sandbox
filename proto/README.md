# Frontal Sandbox Proto

Versioned wire contracts for the sandbox. Packages mirror directories: `sandbox/v1/<area>/` is package `sandbox.v1.<area>`.

- `types/`: shared messages (IDs, hashes, session and exec messages, policy, events, errors). No services.
- `api/`: client-facing gRPC services. The REST API is specified separately, from Rust, in `openapi/`. These services define the gRPC contract, but no gRPC server serves them yet.
- `worker/`: scheduler to worker. Served over a private Unix socket (`sandbox-worker`).
- `guest/`: host to in-VM agent (plans/PLAN.md section 4.10). Schema only, until the VM tier can run.

## Checks

```sh
buf lint proto
buf build proto
```

`buf.yaml` relaxes three RPC naming rules on purpose: RPCs share domain messages and `google.protobuf.Empty`. `buf breaking` needs a previous proto tree in git, so it is only meaningful after the first commit that includes `proto/`.

## Rules

- Never reuse a field number or an enum value. Reserve them when removing.
- Enum zero values are `*_UNSPECIFIED`, and the conversion layer rejects them.
- Use wrapper messages where unset and empty must differ (for example `PortList`).
