# Sandbox Server

HTTP API server for the sandbox system, built on `axum`.

**Status: early.** Session, exec, filesystem, snapshot, and fork routes run against real in-memory sessions. Snapshots capture the committed filesystem root, and forks restore it. Settings the backend does not enforce are refused with `501`. The crate has a library and the `gen_openapi` binary, but no server binary.

## Public API

Re-exported from `src/lib.rs`:

- `create_router(scheduler: Arc<Scheduler>, auth: Arc<AuthManager>) -> axum::Router` (`src/api.rs`)
- `AuthManager` (`src/auth.rs`): `create_key` returns a new bearer key, `validate_api_key` checks one, and `revoke` removes one. Keys are stored as hashes.
- `openapi` (`src/openapi.rs`): `document`, `json`, and `yaml` return the spec.

## Routes

Every `/v1` route requires `Authorization: Bearer <key>`. `GET /v1/openapi.json` is public.

| Method | Path | Behavior |
|--------|------|----------|
| POST | `/v1/sessions` | Creates an isolate-backed session owned by the calling key. Returns `201`. |
| GET | `/v1/sessions/:id` | Returns the session state. |
| DELETE | `/v1/sessions/:id` | Destroys the session. Returns `204`. |
| POST | `/v1/sessions/:id/exec` | Runs one command and returns its buffered output. |
| GET | `/v1/sessions/:id/fs/*path` | Reads a file as `application/octet-stream`. |
| PUT | `/v1/sessions/:id/fs/*path` | Creates or replaces a file. Returns `204`. |
| DELETE | `/v1/sessions/:id/fs/*path` | Deletes a file or an empty directory. Returns `204`. |
| POST | `/v1/sessions/:id/snapshot` | Takes a snapshot of the committed filesystem. Returns `201`. |
| POST | `/v1/snapshots/:id/fork` | Starts `count` sessions (1 to 64) from the snapshot. Returns `200`. |

Errors use the `ErrorBody` shape (`code`, `message`). Status codes follow the error code: `NOT_FOUND` is 404, `INVALID_ARGUMENT` is 400, `UNSUPPORTED` is 501, and so on.

## Limits

| Limit | Value | Over the limit |
|-------|-------|----------------|
| Live sessions per key | 64 | `429`, `RESOURCE_EXHAUSTED` |
| Live sessions across all keys | 4096 | `429`, `RESOURCE_EXHAUSTED` |
| Snapshots per key | 32 | `429`, `RESOURCE_EXHAUSTED` |
| Sessions per fork call | 1 to 64 | `400` |
| File read size | 16 MiB | `413`, `RESOURCE_EXHAUSTED` |
| Request body size | 2 MiB | `413` from the framework |

Snapshots are held in this server's own table, so the per-key limits, the owner checks, and the total cap all apply to the same records. A snapshot holds a reference to the blob store, which is process-wide and is never freed. Blobs are not charged to a key, so the per-key limit bounds the number of snapshots, not the bytes they pin.

## Ownership and paths

- A session belongs to the key that created it. Any other key gets `404`, the same as for a session that does not exist, so the response does not reveal that the session exists.
- A filesystem path must not contain `..`. It is resolved under the session's root, and a `..` segment is refused with `400`.

## Not routed yet

`GET /v1/sessions/:id/ws`, `GET /v1/sessions/:id/trace`, `GET /v1/sessions/:id/fs?list=...`, and `GET /v1/usage` appear in PLAN section 4.15 but have no route. The SDKs call some snapshot GET routes that are not routed.

## Security status

- Sessions are isolated per key, and paths are checked. No other tenant controls are in place.
- No rate limiting, request IDs on responses, or scope checks.
- Do not expose the server to untrusted clients. Key provisioning is in-process only, and there is no TLS termination.

## Spec

The OpenAPI spec is generated from the handler annotations and committed under `openapi/v1/`. The `committed_json_matches_the_code` test fails when the committed files drift. See [docs/adr/0001](../../docs/adr/0001-protobuf-and-openapi-contracts.md).
