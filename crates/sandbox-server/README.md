# Sandbox Server

HTTP API server for the sandbox system, built on `axum`.

**Status: placeholder.** Routes are registered, but every handler returns a fixed string (for example `"create_session"`). Nothing reads or writes sessions, and no request is executed. API key validation is a stub that accepts every key. The crate has no binary target.

## Public API

Re-exported from `src/lib.rs`:

- `create_router(scheduler: Arc<Scheduler>) -> axum::Router` (`src/api.rs`)
- `AuthManager` (`src/auth.rs`): `validate_api_key` always returns `Ok(())`. The code carries `TODO: Implement API key validation`.
- `middleware` (`src/middleware.rs`): `request_id_middleware` (adds a UUID v4 to request extensions) and `logging_middleware` (logs method, URI, status, and duration). Neither is attached to the router.

## Routes registered in `src/api.rs`

| Method | Path | Handler status |
|--------|------|----------------|
| POST | `/v1/sessions` | placeholder |
| GET | `/v1/sessions/:id` | placeholder |
| DELETE | `/v1/sessions/:id` | placeholder |
| POST | `/v1/sessions/:id/exec` | placeholder |
| GET/PUT/DELETE | `/v1/sessions/:id/fs/*path` | placeholder |
| POST | `/v1/sessions/:id/snapshot` | placeholder |
| POST | `/v1/snapshots/:id/fork` | placeholder |

Not routed yet: `GET /v1/sessions/:id/ws`, `GET /v1/sessions/:id/trace`, `GET /v1/sessions/:id/fs?list=...`, and `GET /v1/usage`. The PLAN lists all of them as the target API.

## Security status

- **No authentication.** Every request is accepted. Do not expose this server to any network.
- No rate limiting, request IDs on responses, or scope checks.
- Planned: API keys hashed at rest with a prefix for lookup, per-key scopes and quotas, and OIDC later.

## Example

Not provided. Handlers are placeholders, and the crate has no `main` to bind a listener. The PLAN's `axum::Server::bind` snippet targets an older axum API and is not used here.

## Spec

See [PLAN.md](../../PLAN.md) section 4.15. Planned: OpenAPI generated from types with CI drift checks, rate limits, and Prometheus and OpenTelemetry output.
