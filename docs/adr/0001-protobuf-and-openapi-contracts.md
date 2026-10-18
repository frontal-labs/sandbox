# 0001: Protobuf for internal contracts, generated OpenAPI for REST

## Status
Accepted

## Date
2026-10-14

## Context
The sandbox had no formal wire contract. The scheduler-to-worker channel was a JSON-lines protocol on a child process's stdout, which shared a stream with guest output. The REST API had no spec. plans/PLAN.md section 4.13 calls for gRPC between scheduler and worker, section 4.10 for protobuf frames to the guest agent, and section 4.15 for a generated OpenAPI spec.

## Decision
- Internal and gRPC contracts are protobuf, in `proto/sandbox/v1/{types,api,worker,guest}/`, compiled by `crates/sandbox-proto`. `buf` lints them.
- The worker channel is gRPC over a Unix socket in a directory the host creates with mode 0700. The stdout protocol is removed.
- The REST contract is generated from Rust (`utoipa`), not from protobuf. The Rust types stay the source of truth for the public API. The spec is committed at `openapi/v1/` and served at `GET /v1/openapi.json`. A test fails when the committed files drift.
- Proto to domain conversions live in `crates/sandbox-proto`, and every proto to domain conversion validates its input.

## Consequences
### Positive
- Guest output can no longer reach the protocol stream.
- Two contracts (proto and REST) stay aligned by tests: schema parity against the descriptor set, and route parity against the router.
- REST JSON is unchanged, so the SDKs and existing docs remain valid.

### Negative
- Two contract sources. Changes to a shared type must touch both `proto/` and the REST types.
- Building `sandbox-proto` needs `protoc`.
- The `api` gRPC services are schema only until a gRPC server is built.
- The byte-field JSON encoding (integer arrays) does not match what the SDKs expect. Fixing that is a separate wire-format decision.
