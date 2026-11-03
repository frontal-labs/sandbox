# 0029: Trace encoding returns errors

## Status
Accepted

## Date
2026-11-03

## Context
`Trace::to_jsonl` encoded each header and run line with `serde_json` and called `expect` on the result. A failed encode panicked the host process. Callers include the server's trace endpoint, the state capture that persists sessions, and the CLI trace writer. The plan says no `unwrap` or `expect` outside tests and `main` (PLAN §11, rule 5), and a public API change needs a design note (rule 4).

## Decision
`Trace::to_jsonl` returns `Result<String, serde_json::Error>`. The CLI writer maps the error to an internal `SandboxError`. The server's trace endpoint does the same, and the state capture now returns `Result<Saved, SandboxError>`, so the store propagates a failed encode to the request that caused it, not to a panic. The store's `save` takes a capture that returns `Result`.

Exec NDJSON and WebSocket encoding follow the same rule. An event that cannot be encoded ends the NDJSON body with an I/O error, and the WebSocket closes. Neither panics.

## Consequences
### Positive
- A serialization failure is reported to the caller and does not take the host down.
- The panic-free rule in PLAN §11 holds for the trace and streaming paths, which production code checks with `clippy::unwrap_used` and `clippy::expect_used`.

### Negative
- `to_jsonl` callers outside this workspace must handle a `Result`. The only callers are in this repository.

### Risks
- The serializers in question encode plain structs of strings, integers, and options, so a failure is not expected in practice. The change makes it visible if that assumption ever breaks.

## Alternatives Considered
- Keep `expect` and document the invariant. Rejected: a panic in the server is worse than an error, and the invariant is not enforced by the type system.
- Return an empty string on failure. Rejected: it would silently lose the trace.

## References
- PLAN §11 rules 4 and 5.
- ADR 0007 (trace format), ADR 0008 (durable server state).
