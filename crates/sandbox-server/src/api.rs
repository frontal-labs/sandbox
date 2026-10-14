//! REST handlers for `/v1`. Sessions belong to the API key that created them. A
//! key that did not create a session sees it as not found.

use crate::auth::{AuthManager, KeyId};
use axum::{
    body::Bytes,
    extract::{Path, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Extension, Router,
};
use sandbox_scheduler::Scheduler;
use sandbox_session::session::SessionFs;
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::hash::ContentHash;
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig, SessionId, SessionState,
    SessionStatus, SnapshotId, TenantId,
};
use sandbox_vfs::{OpenFlags, VfsError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use utoipa::ToSchema;

#[derive(Clone)]
pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub auth: Arc<AuthManager>,
    pub sessions: SessionTable,
}

/// Live sessions, keyed by ID, each with the key that owns it.
#[derive(Clone, Default)]
pub struct SessionTable(Arc<RwLock<HashMap<SessionId, Entry>>>);

struct Entry {
    owner: KeyId,
    state: SessionState,
    session: Arc<dyn Session>,
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

/// An error that renders as an [`ErrorBody`] with the status its code implies.
pub struct ApiError(SandboxError);

impl From<SandboxError> for ApiError {
    fn from(error: SandboxError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::InvalidArgument | ErrorCode::ShellError => StatusCode::BAD_REQUEST,
            ErrorCode::AlreadyExists | ErrorCode::InvalidState => StatusCode::CONFLICT,
            ErrorCode::PermissionDenied | ErrorCode::PolicyDenied => StatusCode::FORBIDDEN,
            ErrorCode::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Unsupported => StatusCode::NOT_IMPLEMENTED,
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            code: self.0.code,
            message: self.0.message,
        };
        (status, Json(body)).into_response()
    }
}

/// Maps VFS errors to API errors. The orphan rule keeps this out of `From`.
fn vfs_error(error: VfsError) -> SandboxError {
    match error {
        VfsError::NotFound(m) => SandboxError::not_found(m),
        VfsError::AlreadyExists(m) => SandboxError::already_exists(m),
        VfsError::InvalidPath(m) => SandboxError::invalid_argument(m),
        VfsError::PermissionDenied(m) => SandboxError::permission_denied(m),
        VfsError::QuotaExceeded(m) => SandboxError::resource_exhausted(m),
        VfsError::Unsupported(m) => SandboxError::unsupported(m),
        other => SandboxError::new(ErrorCode::VfsError, other.to_string()),
    }
}

/// Turns a request path into an absolute VFS path. `..` is refused, so a path
/// cannot name anything outside the session's tree.
fn vfs_path(raw: &str) -> Result<PathBuf, ApiError> {
    if raw.split('/').any(|segment| segment == "..") {
        return Err(SandboxError::invalid_argument("path must not contain '..'").into());
    }
    Ok(PathBuf::from(format!("/{raw}")))
}

/// A session the caller owns. A session owned by another key is not found, so
/// the response does not reveal that it exists.
struct Owned {
    session: Arc<dyn Session>,
    state: SessionState,
}

async fn owned(state: &AppState, owner: &KeyId, raw: &str) -> Result<Owned, ApiError> {
    let not_found = || SandboxError::not_found(format!("session {raw} not found"));
    let id: SessionId = raw.parse().map_err(|_| not_found())?;
    let table = state.sessions.0.read().await;
    match table.get(&id) {
        Some(entry) if entry.owner == *owner => Ok(Owned {
            session: entry.session.clone(),
            state: entry.state.clone(),
        }),
        _ => Err(not_found().into()),
    }
}

/// Rejects requests without a valid `Authorization: Bearer <key>` header, and
/// passes the authenticated key to the handlers.
async fn require_api_key(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
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
            request.extensions_mut().insert(id);
            next.run(request).await
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Builds the API router. Every `/v1` route requires a valid bearer API key,
/// except `GET /v1/openapi.json`, which holds only the public contract.
pub fn create_router(scheduler: Arc<Scheduler>, auth: Arc<AuthManager>) -> Router {
    let state = AppState {
        scheduler,
        auth: auth.clone(),
        sessions: SessionTable::default(),
    };

    Router::new()
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id", delete(destroy_session))
        .route("/v1/sessions/:id/exec", post(exec_command))
        .route("/v1/sessions/:id/fs/*path", get(read_file))
        .route("/v1/sessions/:id/fs/*path", axum::routing::put(write_file))
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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::openapi::document())
}

/// Create a session from a config. The session is owned by the calling key.
#[utoipa::path(
    post,
    path = "/v1/sessions",
    tag = "sessions",
    request_body = SessionConfig,
    responses(
        (status = 201, description = "Session created", body = CreateSessionResponse),
        (status = 400, description = "Invalid session config", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn create_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Json(config): Json<SessionConfig>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    let tenant_id: TenantId = config.tenant_id;
    let labels = config.labels.clone();
    let backend = IsolateBackend::new(config).await?;
    let id = backend.id();
    let now = SystemTime::now();
    let session_state = SessionState {
        id,
        tenant_id,
        tier: backend.tier(),
        status: SessionStatus::Running,
        config_hash: ContentHash::nil(),
        vfs_root: None,
        created_at: now,
        updated_at: now,
        labels,
    };
    let entry = Entry {
        owner,
        state: session_state,
        session: Arc::new(backend),
    };
    state.sessions.0.write().await.insert(id, entry);
    Ok((
        StatusCode::CREATED,
        Json(CreateSessionResponse { session_id: id }),
    ))
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
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn get_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, ApiError> {
    Ok(Json(owned(&state, &owner, &id).await?.state))
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
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn destroy_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    // Confirms ownership before removing the entry.
    owned(&state, &owner, &id).await?;
    let session_id: SessionId = id
        .parse()
        .map_err(|_| SandboxError::not_found(format!("session {id} not found")))?;
    state.sessions.0.write().await.remove(&session_id);
    Ok(StatusCode::NO_CONTENT)
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
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn exec_command(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Json(request): Json<ExecRequest>,
) -> Result<Json<ExecResult>, ApiError> {
    let session = owned(&state, &owner, &id).await?.session;
    Ok(Json(session.exec(request).await?))
}

/// Read a file from a session's filesystem.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session, without the leading slash"),
    ),
    responses(
        (status = 200, description = "File contents", content_type = "application/octet-stream"),
        (status = 400, description = "Path is a directory or contains '..'", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session, or no such file", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn read_file(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let session = owned(&state, &owner, &id).await?.session;
    let target = vfs_path(&path)?;
    let fs = session.fs();
    let stat = fs.stat(&target).await.map_err(vfs_error)?;
    if stat.is_dir {
        return Err(SandboxError::invalid_argument("path is a directory").into());
    }
    let handle = fs.open(&target, read_flags()).await.map_err(vfs_error)?;
    let read = fs.read(&handle, 0, stat.size as usize).await;
    fs.close(&handle);
    let bytes = read.map_err(vfs_error)?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes.to_vec(),
    )
        .into_response())
}

/// Write a file in a session's filesystem, creating or replacing it.
#[utoipa::path(
    put,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session, without the leading slash"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204, description = "File written"),
        (status = 400, description = "Path is a directory or contains '..'", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn write_file(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let session = owned(&state, &owner, &id).await?.session;
    let target = vfs_path(&path)?;
    let fs = session.fs();
    let handle = fs.open(&target, write_flags()).await.map_err(vfs_error)?;
    let written = write_all(fs, &handle, &body).await;
    fs.close(&handle);
    written.map_err(vfs_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Writes every byte, since one VFS write may accept only part of the buffer.
async fn write_all(
    fs: &dyn SessionFs,
    handle: &sandbox_vfs::FileHandle,
    data: &[u8],
) -> Result<(), VfsError> {
    let mut offset = 0usize;
    while offset < data.len() {
        let n = fs.write(handle, offset as u64, &data[offset..]).await?;
        if n == 0 {
            break;
        }
        offset += n;
    }
    Ok(())
}

/// Delete a file or an empty directory in a session's filesystem.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{id}/fs/{path}",
    tag = "fs",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("path" = String, Path, description = "Absolute path inside the session, without the leading slash"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Path contains '..', or a non-empty directory", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session, or no such path", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn delete_file(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path((id, path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let session = owned(&state, &owner, &id).await?.session;
    let target = vfs_path(&path)?;
    session.fs().rm(&target, false).await.map_err(vfs_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a snapshot of a session. Returns `501` when the session's backend
/// cannot snapshot, which the isolate backend does not yet do.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/snapshot",
    tag = "snapshots",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 201, description = "Snapshot taken", body = TakeSnapshotResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
        (status = 501, description = "The backend cannot snapshot yet", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn take_snapshot(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<TakeSnapshotResponse>), ApiError> {
    let session = owned(&state, &owner, &id).await?.session;
    let snapshot_id = session.snapshot().await?;
    Ok((
        StatusCode::CREATED,
        Json(TakeSnapshotResponse { snapshot_id }),
    ))
}

/// Fork a snapshot into new sessions. Returns `501` until the backend can
/// restore snapshots, because restoring needs captured filesystem state.
#[utoipa::path(
    post,
    path = "/v1/snapshots/{id}/fork",
    tag = "snapshots",
    params(("id" = String, Path, description = "Snapshot ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ForkRequest,
    responses(
        (status = 200, description = "Sessions created", body = ForkResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 501, description = "Restoring snapshots is not implemented yet", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn fork_snapshot(
    State(_state): State<AppState>,
    Extension(_owner): Extension<KeyId>,
    Path(_id): Path<String>,
    Json(_request): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, ApiError> {
    Err(SandboxError::unsupported(
        "forking snapshots is not implemented: restore needs captured filesystem state",
    )
    .into())
}

fn read_flags() -> OpenFlags {
    OpenFlags {
        read: true,
        write: false,
        append: false,
        create: false,
        truncate: false,
        exclusive: false,
    }
}

fn write_flags() -> OpenFlags {
    OpenFlags {
        read: false,
        write: true,
        append: false,
        create: true,
        truncate: true,
        exclusive: false,
    }
}
