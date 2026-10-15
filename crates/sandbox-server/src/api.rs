//! REST handlers for `/v1`. Sessions belong to the API key that created them. A
//! key that did not create a session sees it as not found.

use crate::auth::{AuthManager, KeyId};
use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
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
use sandbox_types::policy::{NetworkPolicy, ResourceLimits};
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionConfig, SessionId, SessionState,
    SessionStatus, Snapshot, SnapshotId, TenantId, Tier, TierPreference,
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
    pub snapshots: SnapshotTable,
}

/// A snapshot taken through the API. Only the key that took it can see it, and
/// the root is the filesystem state it restores.
#[derive(Clone)]
struct SnapshotRecord {
    owner: KeyId,
    session: SessionId,
    root: ContentHash,
    created_at: SystemTime,
    labels: HashMap<String, String>,
}

/// Snapshots held by the server, keyed by ID. The server owns this table, so
/// the snapshot limits and the owner checks live in one place.
#[derive(Clone, Default)]
pub struct SnapshotTable {
    records: Arc<RwLock<HashMap<SnapshotId, SnapshotRecord>>>,
    /// Serializes snapshot admission, so the limits cannot be raced past.
    admit: Arc<tokio::sync::Mutex<()>>,
}

/// Most sessions one fork call may create.
const MAX_FORK_COUNT: u32 = 64;
/// Live sessions one key may hold. Sessions use process memory, so this bounds
/// what a single key can take from the server.
pub const MAX_SESSIONS_PER_KEY: usize = 64;
/// Live sessions the server holds across all keys.
pub const MAX_SESSIONS_TOTAL: usize = 4096;
/// Largest file one read returns. A read holds the whole file in memory, and a
/// session may hold files up to the VFS quota of 100 MiB, so reads are capped.
pub const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
/// Largest request body. Writes are bounded by this, not by the VFS quota.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Snapshots one key may hold. Snapshots pin blobs in the shared store, and
/// the process-wide snapshot registry is capped separately.
pub const MAX_SNAPSHOTS_PER_KEY: usize = 32;
/// Snapshots the server holds across all keys.
pub const MAX_SNAPSHOTS_TOTAL: usize = 4096;

/// Stores a new session if the caller and the server are under their limits.
/// The check and the insert happen under one lock, so concurrent creates
/// cannot both pass a limit that only one of them should reach.
async fn admit_session(state: &AppState, owner: &KeyId, entry: Entry) -> Result<(), ApiError> {
    let id = entry.state.id;
    let mut table = state.sessions.0.write().await;
    if table.len() >= MAX_SESSIONS_TOTAL {
        return Err(SandboxError::resource_exhausted("the server is at its session limit").into());
    }
    let held = table.values().filter(|e| e.owner == *owner).count();
    if held >= MAX_SESSIONS_PER_KEY {
        return Err(SandboxError::resource_exhausted(format!(
            "this key already holds {MAX_SESSIONS_PER_KEY} sessions"
        ))
        .into());
    }
    table.insert(id, entry);
    Ok(())
}

/// Live sessions, keyed by ID, each with the key that owns it.
#[derive(Clone, Default)]
pub struct SessionTable(Arc<RwLock<HashMap<SessionId, Entry>>>);

struct Entry {
    owner: KeyId,
    state: SessionState,
    session: Arc<dyn Session>,
    /// Held for every command, file operation, and snapshot on this session.
    /// A checkpoint must not interleave with a write, or the write can be lost
    /// from the snapshot or from the overlay.
    gate: Arc<tokio::sync::Mutex<()>>,
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

/// Whether a file of `size` bytes is too large to read in one call.
fn over_read_limit(size: u64) -> bool {
    size > MAX_READ_BYTES
}

/// Turns a request path into an absolute VFS path. `..` is refused, so a path
/// cannot name anything outside the session's tree.
fn vfs_path(raw: &str) -> Result<PathBuf, ApiError> {
    if raw.split('/').any(|segment| segment == "..") {
        return Err(SandboxError::invalid_argument("path must not contain '..'").into());
    }
    Ok(PathBuf::from(format!("/{raw}")))
}

/// Whether `requested` sets a limit to a value other than the default. A field
/// left unset means the default, so `{}` requests no custom limits.
fn requests_custom_limits(requested: &ResourceLimits) -> bool {
    let d = ResourceLimits::default();
    (requested.memory_bytes.is_some() && requested.memory_bytes != d.memory_bytes)
        || (requested.cpu_ms_per_sec.is_some() && requested.cpu_ms_per_sec != d.cpu_ms_per_sec)
        || (requested.wall_timeout_secs.is_some()
            && requested.wall_timeout_secs != d.wall_timeout_secs)
        || (requested.max_file_size.is_some() && requested.max_file_size != d.max_file_size)
        || (requested.max_total_size.is_some() && requested.max_total_size != d.max_total_size)
        || (requested.max_inodes.is_some() && requested.max_inodes != d.max_inodes)
        || (requested.max_path_depth.is_some() && requested.max_path_depth != d.max_path_depth)
        || (requested.max_processes.is_some() && requested.max_processes != d.max_processes)
        || (requested.max_open_files.is_some() && requested.max_open_files != d.max_open_files)
}

/// Refuses a config that asks for something the backend does not enforce.
/// Ignoring such a setting would look like success while giving no protection.
fn refuse_unenforced(config: &SessionConfig) -> Result<(), ApiError> {
    let refuse = |what: &str| {
        Err(SandboxError::unsupported(format!(
            "{what} is not enforced yet, so the request is refused rather than ignored"
        ))
        .into())
    };
    if !matches!(
        config.tier_preference,
        TierPreference::Auto | TierPreference::Isolate
    ) {
        return refuse("the vm tier");
    }
    if requests_custom_limits(&config.limits) {
        return refuse("per-session resource limits");
    }
    if config.network != NetworkPolicy::default() {
        return refuse("a network policy");
    }
    if config.secrets.is_some() {
        return refuse("secrets");
    }
    if !config.env.is_empty() {
        return refuse("environment variables");
    }
    if config.cwd != SessionConfig::default().cwd {
        return refuse("a working directory");
    }
    Ok(())
}

/// Wraps a live session with its metadata and owner.
fn entry(
    owner: KeyId,
    session: Arc<dyn Session>,
    tenant_id: TenantId,
    labels: HashMap<String, String>,
) -> Entry {
    let now = SystemTime::now();
    let state = SessionState {
        id: session.id(),
        tenant_id,
        tier: Tier::Isolate,
        status: SessionStatus::Running,
        config_hash: ContentHash::nil(),
        vfs_root: None,
        created_at: now,
        updated_at: now,
        labels,
    };
    Entry {
        owner,
        state,
        session,
        gate: Arc::new(tokio::sync::Mutex::new(())),
    }
}

/// A session the caller owns. A session owned by another key is not found, so
/// the response does not reveal that it exists.
struct Owned {
    session: Arc<dyn Session>,
    state: SessionState,
    gate: Arc<tokio::sync::Mutex<()>>,
}

async fn owned(state: &AppState, owner: &KeyId, raw: &str) -> Result<Owned, ApiError> {
    let not_found = || SandboxError::not_found(format!("session {raw} not found"));
    let id: SessionId = raw.parse().map_err(|_| not_found())?;
    let table = state.sessions.0.read().await;
    match table.get(&id) {
        Some(entry) if entry.owner == *owner => Ok(Owned {
            session: entry.session.clone(),
            state: entry.state.clone(),
            gate: entry.gate.clone(),
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
        snapshots: SnapshotTable::default(),
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
        .route("/v1/sessions/:id/snapshots", get(list_snapshots))
        .route("/v1/sessions/:id/snapshots/:snapshot_id", get(get_snapshot))
        .route("/v1/snapshots/:id/fork", post(fork_snapshot))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .route("/v1/openapi.json", get(openapi_json))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
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
        (status = 404, description = "initial_snapshot is not one of this key's snapshots", body = ErrorBody),
        (status = 501, description = "A requested setting is not enforced yet, so the request is refused", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn create_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Json(config): Json<SessionConfig>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    refuse_unenforced(&config)?;
    let tenant_id = config.tenant_id;
    let labels = config.labels.clone();
    let session: Arc<dyn Session> = match config.initial_snapshot {
        Some(snapshot_id) => {
            let record = snapshot_for(&state, &owner, snapshot_id).await?;
            sandbox_session::restore(record.root).await?.into()
        }
        None => Arc::new(IsolateBackend::new(config).await?),
    };
    let id = session.id();
    let entry = entry(owner.clone(), session, tenant_id, labels);
    admit_session(&state, &owner, entry).await?;
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
        (status = 501, description = "Streaming exec is not implemented yet", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn exec_command(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Query(query): Query<ExecQuery>,
    Json(request): Json<ExecRequest>,
) -> Result<Json<ExecResult>, ApiError> {
    // Output is buffered. Streaming would need the backend to emit chunks while
    // the command runs, which it cannot do yet, so refuse rather than fake it.
    if query.stream.unwrap_or(false) || request.stream {
        return Err(SandboxError::unsupported("streaming exec is not implemented yet").into());
    }
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let session = own.session;
    Ok(Json(session.exec(request).await?))
}

/// Query options for exec.
#[derive(Debug, Deserialize)]
pub struct ExecQuery {
    /// Streaming output. Not implemented, so `true` is refused with 501.
    pub stream: Option<bool>,
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
        (status = 413, description = "File is larger than the 16 MiB read limit", body = ErrorBody),
        (status = 404, description = "No such session, or no such file", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn read_file(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let session = own.session;
    let target = vfs_path(&path)?;
    let fs = session.fs();
    let stat = fs.stat(&target).await.map_err(vfs_error)?;
    if stat.is_dir {
        return Err(SandboxError::invalid_argument("path is a directory").into());
    }
    if over_read_limit(stat.size) {
        let body = ErrorBody {
            code: ErrorCode::ResourceExhausted,
            message: format!(
                "file is {} bytes; reads are limited to {MAX_READ_BYTES} bytes",
                stat.size
            ),
        };
        return Ok((StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response());
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
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let session = own.session;
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
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let session = own.session;
    let target = vfs_path(&path)?;
    session.fs().rm(&target, false).await.map_err(vfs_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a snapshot of a session's committed filesystem. The caller owns the
/// snapshot, and only that key can fork it or start a session from it.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/snapshot",
    tag = "snapshots",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 201, description = "Snapshot taken", body = TakeSnapshotResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn take_snapshot(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<TakeSnapshotResponse>), ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let session = own.session;
    let _admit = state.snapshots.admit.lock().await;
    {
        let records = state.snapshots.records.read().await;
        if records.len() >= MAX_SNAPSHOTS_TOTAL {
            return Err(
                SandboxError::resource_exhausted("the server is at its snapshot limit").into(),
            );
        }
        let held = records.values().filter(|r| r.owner == owner).count();
        if held >= MAX_SNAPSHOTS_PER_KEY {
            return Err(SandboxError::resource_exhausted(format!(
                "this key already holds {MAX_SNAPSHOTS_PER_KEY} snapshots"
            ))
            .into());
        }
    }
    let root = session.snapshot().await?;
    let snapshot_id = SnapshotId::new();
    let record = SnapshotRecord {
        owner,
        session: own.state.id,
        root,
        created_at: SystemTime::now(),
        labels: own.state.labels.clone(),
    };
    state
        .snapshots
        .records
        .write()
        .await
        .insert(snapshot_id, record);
    Ok((
        StatusCode::CREATED,
        Json(TakeSnapshotResponse { snapshot_id }),
    ))
}

/// Fork a snapshot into new sessions, each owned by the caller. The caller must
/// own the snapshot.
#[utoipa::path(
    post,
    path = "/v1/snapshots/{id}/fork",
    tag = "snapshots",
    params(("id" = String, Path, description = "Snapshot ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ForkRequest,
    responses(
        (status = 200, description = "Sessions created", body = ForkResponse),
        (status = 400, description = "Count must be between 1 and 64", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such snapshot for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn fork_snapshot(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Json(request): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, ApiError> {
    if request.count == 0 || request.count > MAX_FORK_COUNT {
        return Err(SandboxError::invalid_argument(format!(
            "count must be between 1 and {MAX_FORK_COUNT}"
        ))
        .into());
    }
    let snapshot_id = parse_snapshot_id(&id)?;
    let record = snapshot_for(&state, &owner, snapshot_id).await?;

    let mut session_ids = Vec::with_capacity(request.count as usize);
    for _ in 0..request.count {
        let session: Arc<dyn Session> = sandbox_session::restore(record.root).await?.into();
        let session_id = session.id();
        let entry = entry(owner.clone(), session, TenantId::nil(), HashMap::new());
        admit_session(&state, &owner, entry).await?;
        session_ids.push(session_id);
    }
    Ok(Json(ForkResponse { session_ids }))
}

fn parse_snapshot_id(raw: &str) -> Result<SnapshotId, ApiError> {
    raw.parse()
        .map_err(|_| SandboxError::not_found(format!("snapshot {raw} not found")).into())
}

/// The snapshot, if the caller took it. Otherwise `NotFound`, which does not
/// reveal that the snapshot exists.
async fn snapshot_for(
    state: &AppState,
    owner: &KeyId,
    snapshot_id: SnapshotId,
) -> Result<SnapshotRecord, ApiError> {
    let records = state.snapshots.records.read().await;
    match records.get(&snapshot_id) {
        Some(record) if record.owner == *owner => Ok(record.clone()),
        _ => Err(SandboxError::not_found(format!("snapshot {snapshot_id} not found")).into()),
    }
}

/// The API view of a snapshot. The VFS root is the only state a snapshot
/// captures, so `env` is empty and `cwd` is the root directory.
fn snapshot_view(id: SnapshotId, record: &SnapshotRecord) -> Snapshot {
    Snapshot {
        id,
        parent: None,
        vfs_root: record.root,
        env: HashMap::new(),
        cwd: "/".to_string(),
        tier: Tier::Isolate,
        runtime_blob: None,
        created_at: record.created_at,
        labels: record.labels.clone(),
    }
}

/// List the snapshots taken from one session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/snapshots",
    tag = "snapshots",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "Snapshots taken from this session", body = Vec<Snapshot>),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn list_snapshots(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Snapshot>>, ApiError> {
    let session_id = owned(&state, &owner, &id).await?.state.id;
    let records = state.snapshots.records.read().await;
    let mut snapshots: Vec<Snapshot> = records
        .iter()
        .filter(|(_, r)| r.owner == owner && r.session == session_id)
        .map(|(snapshot_id, r)| snapshot_view(*snapshot_id, r))
        .collect();
    snapshots.sort_by_key(|s| s.created_at);
    Ok(Json(snapshots))
}

/// Read one snapshot taken from a session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/snapshots/{snapshot_id}",
    tag = "snapshots",
    params(
        ("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
        ("snapshot_id" = String, Path, description = "Snapshot ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$"),
    ),
    responses(
        (status = 200, description = "The snapshot", body = Snapshot),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session or snapshot for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn get_snapshot(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path((id, snapshot_id)): Path<(String, String)>,
) -> Result<Json<Snapshot>, ApiError> {
    let session_id = owned(&state, &owner, &id).await?.state.id;
    let snapshot_id = parse_snapshot_id(&snapshot_id)?;
    let records = state.snapshots.records.read().await;
    match records.get(&snapshot_id) {
        Some(record) if record.owner == owner && record.session == session_id => {
            Ok(Json(snapshot_view(snapshot_id, record)))
        }
        _ => Err(SandboxError::not_found(format!("snapshot {snapshot_id} not found")).into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_capped_at_the_limit() {
        assert!(!over_read_limit(0));
        assert!(!over_read_limit(MAX_READ_BYTES));
        assert!(over_read_limit(MAX_READ_BYTES + 1));
        // The VFS allows 100 MiB per file, which is above the read cap.
        assert!(over_read_limit(100 * 1024 * 1024));
    }
}
