# Sandbox Proto

Generated protobuf and gRPC bindings for the sandbox wire contracts, plus the conversions to and from `sandbox-types`.

## Status

Working, with unit, property, and contract tests (`tests/roundtrip.rs`, `tests/compat.rs`, `src/framing.rs`). The worker channel uses it in `sandbox-worker`. The guest and public `api` services are schema only.

- Source: `proto/sandbox/v1/{types,api,worker,guest}/` at the repository root. `build.rs` compiles them with `tonic-build` (needs `protoc` on PATH or `PROTOC` set). The Bazel target uses the same build script with protoc from the `protobuf` module.
- `sandbox_proto::v1::{types, api, worker, guest}`: generated messages and services.
- `convert/`: `From<domain>` for proto is infallible. `TryFrom<proto>` validates every field and returns `InvalidArgument` on bad input. An empty port list stays distinct from "all ports". Timestamps keep pre-epoch values.
- `status`: `SandboxError` to `tonic::Status`. The exact `ErrorCode` travels in the `sandbox-error-code` metadata key, so the round trip is lossless.
- `framing`: 4-byte big-endian length prefix, capped at 16 MiB before any allocation.
- `FILE_DESCRIPTOR_SET`: the compiled schema, used by the parity test.

## Tests

- Round-trip properties for every message family, and hostile-input rejection (malformed IDs, short hashes, unspecified or unknown enums, out-of-range ports, empty argv, missing required fields).
- Parity: each domain type's serde fields match its proto message's fields, read from the compiled descriptor set.

## Adding a message

Add it to a `.proto` file, add the conversion in `src/convert/`, add it to the round-trip property tests, and run `buf lint proto`. Never reuse a field number or an enum value.

## License

Apache-2.0
