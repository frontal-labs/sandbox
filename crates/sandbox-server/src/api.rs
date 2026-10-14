use crate::auth::{AuthManager, KeyId};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post, put},
    Router,
};
use sandbox_scheduler::Scheduler;
use sandbox_types::{ErrorCode, SessionId};
// The sandbox_types names below appear only inside #[utoipa::path] attributes.
use sandbox_types::SnapshotId;
#[allow(unused_imports)]
use sandbox_types::{ExecRequest, ExecResult, SessionConfig, SessionState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Clone)]
pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub auth: Arc<AuthManager>,
}

/// Error body for every non-2xx response on `/v1`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionResponse {
    #[schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")]
    pub session_id: SessionId,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TakeSnapshotResponse {
    #[schema(value_type = String, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")]
    pub snapshot_id: SnapshotId,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ForkRequest {
    /// Number of sessions to create from the snapshot.
    pub count: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ForkResponse {
    #[schema(value_type = Vec<String>, pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")]
    pub session_ids: Vec<SessionId>,
}

/// Builds the API router. Every `/v1` route requires a valid bearer API key,
/// except `GET /v1/openapi.json`, which holds only the public contract.
///
/// The handlers are placeholders and answer `501 Not Implemented`. They do not
/// report success for work that was not done.
pub fn create_router(scheduler: Arc<Scheduler>, auth: Arc<AuthManager>) -> Router {
    let state = AppState {
        scheduler,
        auth: auth.clone(),
    };

    Router::new()
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id", delete(destroy_session))
        .route("/v1/sessions/:id/exec", post(exec_command))
        .route("/v1/sessions/:id/fs/*path", get(read_file))
        .route("/v1/sessions/:id/fs/*path", put(write_file))
        .route("/v1/sessions/:id/fs/*path", delete(delete_file))
        .route("/v1/sessions/:id/snapshot", post(take_snapshot))
        .route("/v1/snapshots/:id/fork", post(fork_snapshot))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .route("/v1/openapi.json", get(openapi_json))
        .with_state(state)
}

/// Rejects requests without a valid `Authorization: Bearer <key>` header.
async fn require_api_key(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let Some(key) = presented else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    match state.auth.validate_api_key(key) {
        Ok(id) => {
            tracing::debug!(key_id = %KeyId::as_str(&id), "request authenticated");
            next.run(request).await
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::openapi::document())
}

/// Create a session from a config.
#[utoipa::path(
    post,
    path = "/v1/sessions",
    tag = "sessions",
    request_body = SessionConfig,
    responses(
        (status = 201, description = "Session created", body = CreateSessionResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn create_session() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Read a session's state.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "Session state", body = SessionState),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn get_session() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Destroy a session.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{id}",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 204, description = "Session destroyed"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn destroy_session() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Run one command in a session.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/exec",
    tag = "exec",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ExecRequest,
    responses(
        (status = 200, description = "Command finished", body = ExecResult),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn exec_command() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Read a file from a session's filesystem.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session"),
    ),
    responses(
        (status = 200, description = "File contents", content_type = "application/octet-stream"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn read_file() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Write a file in a session's filesystem.
#[utoipa::path(
    put,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204, description = "File written"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn write_file() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Delete a file or directory in a session's filesystem.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn delete_file() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Take a snapshot of a session.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/snapshot",
    tag = "snapshots",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 201, description = "Snapshot taken", body = TakeSnapshotResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn take_snapshot() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Fork a snapshot into new sessions.
#[utoipa::path(
    post,
    path = "/v1/snapshots/{id}/fork",
    tag = "snapshots",
    params(("id" = String, Path, description = "Snapshot ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ForkRequest,
    responses(
        (status = 200, description = "Sessions created", body = ForkResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Not implemented yet (placeholder)", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn fork_snapshot() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
