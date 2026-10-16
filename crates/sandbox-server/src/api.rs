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
use sandbox_session::{restore_checkpoint, Checkpoint, IsolateBackend, Session};
use sandbox_trace::{outcome_of, RunRecord, Trace};
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
    /// Where keys, snapshots, and sleeping sessions are saved. `None` keeps the
    /// server in memory (ADR 0008).
    pub store: Option<Arc<crate::persist::Store>>,
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
    /// `None` while the session sleeps. Sleep drops the live session, so a
    /// sleeping session holds no VFS in memory. See ADR 0006.
    session: Option<Arc<dyn Session>>,
    /// The session's last checkpoint: what a restart or a wake brings back. Only
    /// a durable server keeps one (ADR 0010).
    checkpoint: Option<Checkpoint>,
    /// Commands the session ran, in order (ADR 0009).
    trace: Vec<RunRecord>,
    /// Bytes the trace holds in its scripts and outputs.
    trace_bytes: usize,
    /// False once a command could not be recorded faithfully, or the trace is full.
    trace_complete: bool,
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
        session: Some(session),
        checkpoint: None,
        trace: Vec::new(),
        trace_bytes: 0,
        trace_complete: true,
        gate: Arc::new(tokio::sync::Mutex::new(())),
    }
}

/// A session the caller owns. A session owned by another key is not found, so
/// the response does not reveal that it exists.
/// A session the caller owns. `session` is `None` while it sleeps.
struct Lookup {
    session: Option<Arc<dyn Session>>,
    state: SessionState,
    gate: Arc<tokio::sync::Mutex<()>>,
}

/// A live session the caller owns.
struct Owned {
    session: Arc<dyn Session>,
    state: SessionState,
    gate: Arc<tokio::sync::Mutex<()>>,
}

/// Finds a session the caller owns, sleeping or not. Other keys get the same
/// not-found answer as a missing session.
async fn lookup(state: &AppState, owner: &KeyId, raw: &str) -> Result<Lookup, ApiError> {
    let not_found = || SandboxError::not_found(format!("session {raw} not found"));
    let id: SessionId = raw.parse().map_err(|_| not_found())?;
    let table = state.sessions.0.read().await;
    match table.get(&id) {
        Some(entry) if entry.owner == *owner => Ok(Lookup {
            session: entry.session.clone(),
            state: entry.state.clone(),
            gate: entry.gate.clone(),
        }),
        _ => Err(not_found().into()),
    }
}

/// Finds a live session the caller owns. A sleeping session is refused until
/// it is woken.
async fn owned(state: &AppState, owner: &KeyId, raw: &str) -> Result<Owned, ApiError> {
    let found = lookup(state, owner, raw).await?;
    match found.session {
        Some(session) => Ok(Owned {
            session,
            state: found.state,
            gate: found.gate,
        }),
        None => Err(asleep().into()),
    }
}

fn asleep() -> SandboxError {
    SandboxError::new(
        ErrorCode::InvalidState,
        "the session is asleep; wake it before using it",
    )
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
    build_router(AppState {
        scheduler,
        auth,
        sessions: SessionTable::default(),
        snapshots: SnapshotTable::default(),
        store: None,
    })
}

/// Opens a durable server whose state lives in `dir` (ADR 0008). Keys, snapshots,
/// and sleeping sessions from an earlier run are loaded. On the first run one key
/// is created, and it is returned so the caller can show it once.
pub async fn open_durable(
    scheduler: Arc<Scheduler>,
    dir: &std::path::Path,
) -> Result<(Router, Option<String>), SandboxError> {
    let (store, saved) = crate::persist::Store::open(dir)?;
    let auth = Arc::new(AuthManager::new());
    let new_key = if saved.keys.is_empty() {
        Some(auth.create_key())
    } else {
        let records: Vec<(String, String)> = saved
            .keys
            .iter()
            .map(|k| (k.id.clone(), k.digest.clone()))
            .collect();
        auth.import_digests(&records)?;
        None
    };
    let state = AppState {
        scheduler,
        auth,
        sessions: SessionTable::default(),
        snapshots: SnapshotTable::default(),
        store: Some(Arc::new(store)),
    };
    restore(&state, saved).await?;
    persist(&state).await.map_err(|e| e.0)?;
    Ok((build_router(state), new_key))
}

/// Loads snapshot records and sleeping sessions into an empty server.
async fn restore(state: &AppState, saved: crate::persist::Saved) -> Result<(), SandboxError> {
    {
        let mut records = state.snapshots.records.write().await;
        for s in saved.snapshots {
            records.insert(
                s.id,
                SnapshotRecord {
                    owner: KeyId::from_stored(s.owner),
                    session: s.session,
                    root: s.root,
                    created_at: s.created_at,
                    labels: s.labels,
                },
            );
        }
    }
    // Version 1 kept sleeping sessions in their own list, with no shell state.
    let legacy = saved
        .sleeping
        .into_iter()
        .map(|s| crate::persist::SavedSession {
            owner: s.owner,
            state: s.state,
            root: s.sleep_root,
            cwd: "/".to_string(),
            env: HashMap::new(),
            sleeping: true,
            trace: s.trace,
            trace_complete: s.trace_complete,
        });
    let mut table = state.sessions.0.write().await;
    for saved_session in saved.sessions.into_iter().chain(legacy) {
        let trace = Trace::parse(&saved_session.trace).map_err(|e| {
            SandboxError::internal(format!("a session's saved trace is damaged: {e}"))
        })?;
        let checkpoint = Checkpoint {
            root: saved_session.root,
            cwd: saved_session.cwd,
            env: saved_session.env,
        };
        // A live session comes back running from its checkpoint. A sleeping one
        // waits for a wake.
        let session: Option<Arc<dyn Session>> = if saved_session.sleeping {
            None
        } else {
            Some(restore_checkpoint(checkpoint.clone()).await?.into())
        };
        let id = saved_session.state.id;
        table.insert(
            id,
            Entry {
                owner: KeyId::from_stored(saved_session.owner),
                state: saved_session.state,
                session,
                checkpoint: Some(checkpoint),
                trace_bytes: trace_size(&trace.runs),
                trace: trace.runs,
                trace_complete: saved_session.trace_complete,
                gate: Arc::new(tokio::sync::Mutex::new(())),
            },
        );
    }
    Ok(())
}

/// The durable state as it is now.
async fn capture(state: &AppState) -> crate::persist::Saved {
    use crate::persist::{Saved, SavedKey, SavedSession, SavedSnapshot};
    let keys = state
        .auth
        .export_digests()
        .into_iter()
        .map(|(id, digest)| SavedKey { id, digest })
        .collect();
    let snapshots = {
        let records = state.snapshots.records.read().await;
        records
            .iter()
            .map(|(id, r)| SavedSnapshot {
                id: *id,
                owner: r.owner.as_str().to_string(),
                session: r.session,
                root: r.root,
                created_at: r.created_at,
                labels: r.labels.clone(),
            })
            .collect()
    };
    let sessions = {
        let table = state.sessions.0.read().await;
        table
            .values()
            .filter_map(|e| {
                // An entry without a checkpoint has no state to bring back.
                let checkpoint = e.checkpoint.as_ref()?;
                Some(SavedSession {
                    owner: e.owner.as_str().to_string(),
                    state: e.state.clone(),
                    root: checkpoint.root,
                    cwd: checkpoint.cwd.clone(),
                    // Environment values are often secrets, so they are never written
                    // to disk. A restored session starts with an empty environment (ADR 0010).
                    env: HashMap::new(),
                    sleeping: e.session.is_none(),
                    trace: Trace {
                        runs: e.trace.clone(),
                    }
                    .to_jsonl(),
                    trace_complete: e.trace_complete,
                })
            })
            .collect()
    };
    Saved {
        version: crate::persist::VERSION,
        keys,
        snapshots,
        sleeping: Vec::new(),
        sessions,
    }
}

/// Records the session's checkpoint on its entry, so the entry can be saved and
/// woken. A memory-only server keeps none, so it skips the cost.
async fn stamp(
    state: &AppState,
    entry: &mut Entry,
    session: &Arc<dyn Session>,
) -> Result<(), ApiError> {
    if state.store.is_some() {
        entry.checkpoint = Some(session.checkpoint().await?);
    }
    Ok(())
}

/// Saves a session after a change, so a restart brings it back as it is now
/// (ADR 0010). The caller holds the session's gate.
async fn save_session(
    state: &AppState,
    id: SessionId,
    session: &Arc<dyn Session>,
) -> Result<(), ApiError> {
    if state.store.is_none() {
        return Ok(());
    }
    let checkpoint = session.checkpoint().await?;
    {
        let mut table = state.sessions.0.write().await;
        if let Some(entry) = table.get_mut(&id) {
            entry.checkpoint = Some(checkpoint);
        }
    }
    persist(state).await
}

/// Saves the durable state, if this server has a store. Call it after the change
/// is in memory and no table lock is held.
async fn persist(state: &AppState) -> Result<(), ApiError> {
    if let Some(store) = &state.store {
        store.save(|| capture(state)).await?;
    }
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id", delete(destroy_session))
        .route("/v1/sessions/:id/exec", post(exec_command))
        .route("/v1/sessions/:id/fs/*path", get(read_file))
        .route("/v1/sessions/:id/fs/*path", axum::routing::put(write_file))
        .route("/v1/sessions/:id/fs/*path", delete(delete_file))
        .route("/v1/sessions/:id/trace", get(get_trace))
        .route("/v1/sessions/:id/sleep", post(sleep_session))
        .route("/v1/sessions/:id/wake", post(wake_session))
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
    let mut entry = entry(owner.clone(), session.clone(), tenant_id, labels);
    stamp(&state, &mut entry, &session).await?;
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
    Ok(Json(lookup(&state, &owner, &id).await?.state))
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
    // Confirms ownership before removing the entry. A sleeping session can be
    // destroyed without waking it.
    lookup(&state, &owner, &id).await?;
    let session_id: SessionId = id
        .parse()
        .map_err(|_| SandboxError::not_found(format!("session {id} not found")))?;
    state.sessions.0.write().await.remove(&session_id);
    persist(&state).await?;
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
    let result = session.exec(request.clone()).await;
    record_exec(&state, own.state.id, &request, &result).await;
    save_session(&state, own.state.id, &session).await?;
    Ok(Json(result?))
}

/// Most runs one session's trace holds. Runs past this are not recorded, and the
/// trace reports itself incomplete, so a long session never fails over its trace.
pub const MAX_TRACE_RUNS: usize = 4096;
/// Most bytes of scripts and output one session's trace holds. Output is the
/// bulk of a trace, and a session's trace is kept in memory and saved with the
/// state, so the budget bounds both.
pub const MAX_TRACE_BYTES: usize = 8 * 1024 * 1024;

/// Bytes a run takes in a trace: its script and its output.
fn run_size(run: &RunRecord) -> usize {
    let outcome = match &run.outcome {
        sandbox_trace::Outcome::Completed { stdout, stderr, .. } => stdout.len() + stderr.len(),
        sandbox_trace::Outcome::Failed { message, .. } => message.len(),
    };
    run.script.len() + outcome
}

fn trace_size(runs: &[RunRecord]) -> usize {
    runs.iter().map(run_size).sum()
}

/// Appends one command to the session's trace (ADR 0009).
async fn record_exec(
    state: &AppState,
    id: SessionId,
    request: &ExecRequest,
    result: &Result<ExecResult, SandboxError>,
) {
    if request.argv.is_empty() {
        return;
    }
    let mut table = state.sessions.0.write().await;
    let Some(entry) = table.get_mut(&id) else {
        return;
    };
    // Standard input cannot be written into a script, so the trace is marked
    // incomplete rather than recording a command that replays differently.
    let script = match script_for(request) {
        Some(script) if request.stdin.as_ref().is_none_or(|s| s.is_empty()) => script,
        _ => {
            entry.trace_complete = false;
            return;
        }
    };
    if entry.trace.len() >= MAX_TRACE_RUNS {
        entry.trace_complete = false;
        return;
    }
    let outcome = outcome_of(
        result
            .as_ref()
            .map(|out| (out.exit_code, out.stdout.clone(), out.stderr.clone()))
            .map_err(|e| SandboxError::new(e.code, e.message.clone())),
    );
    let seq = entry.trace.len() as u64 + 1;
    let run = RunRecord {
        seq,
        script,
        outcome,
    };
    let size = run_size(&run);
    if entry.trace_bytes + size > MAX_TRACE_BYTES {
        entry.trace_complete = false;
        return;
    }
    entry.trace_bytes += size;
    entry.trace.push(run);
}

/// The script that runs `request` in a shell: its directory, then its argv, with
/// every word single-quoted so the arguments keep their meaning. `None` when the
/// request sets environment variables, because their values are not recorded.
fn script_for(request: &ExecRequest) -> Option<String> {
    fn quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    // Environment values are often secrets, so a command that sets them is not
    // recorded. Its trace is marked incomplete instead, and no value is written.
    if !request.env.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(cwd) = &request.cwd {
        parts.push(format!("cd {}", quote(cwd)));
    }
    parts.push(
        request
            .argv
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
    );
    Some(parts.join("; "))
}

/// The session's trace, one run per line, in the ADR 0007 format (ADR 0009).
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/trace",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "The trace. X-Trace-Complete is false when a command could not be recorded or the trace is full", body = String, content_type = "application/x-ndjson"),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn get_trace(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let found = lookup(&state, &owner, &id).await?;
    let (runs, complete) = {
        let table = state.sessions.0.read().await;
        let entry = table
            .get(&found.state.id)
            .ok_or_else(|| SandboxError::not_found(format!("session {id} not found")))?;
        (entry.trace.clone(), entry.trace_complete)
    };
    let body = Trace { runs }.to_jsonl();
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (
                header::HeaderName::from_static("x-trace-complete"),
                if complete { "true" } else { "false" },
            ),
        ],
        body,
    )
        .into_response())
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
    save_session(&state, own.state.id, &session).await?;
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
    save_session(&state, own.state.id, &session).await?;
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
    let _admit = admit_snapshot(&state, &owner).await?;
    let root = own.session.snapshot().await?;
    let snapshot_id = store_snapshot(&state, owner, &own.state, root).await;
    persist(&state).await?;
    Ok((
        StatusCode::CREATED,
        Json(TakeSnapshotResponse { snapshot_id }),
    ))
}

/// Takes the snapshot admission lock and checks the limits. Hold the returned
/// guard until the record is stored, so two requests cannot both pass the check.
async fn admit_snapshot<'a>(
    state: &'a AppState,
    owner: &KeyId,
) -> Result<tokio::sync::MutexGuard<'a, ()>, ApiError> {
    let admit = state.snapshots.admit.lock().await;
    let records = state.snapshots.records.read().await;
    if records.len() >= MAX_SNAPSHOTS_TOTAL {
        return Err(SandboxError::resource_exhausted("the server is at its snapshot limit").into());
    }
    let held = records.values().filter(|r| r.owner == *owner).count();
    if held >= MAX_SNAPSHOTS_PER_KEY {
        return Err(SandboxError::resource_exhausted(format!(
            "this key already holds {MAX_SNAPSHOTS_PER_KEY} snapshots"
        ))
        .into());
    }
    Ok(admit)
}

/// Stores a snapshot record for a checkpointed root and returns its ID.
async fn store_snapshot(
    state: &AppState,
    owner: KeyId,
    session: &SessionState,
    root: ContentHash,
) -> SnapshotId {
    let snapshot_id = SnapshotId::new();
    let record = SnapshotRecord {
        owner,
        session: session.id,
        root,
        created_at: SystemTime::now(),
        labels: session.labels.clone(),
    };
    state
        .snapshots
        .records
        .write()
        .await
        .insert(snapshot_id, record);
    snapshot_id
}

/// Put a session to sleep. The filesystem is checkpointed and recorded as a
/// snapshot, and the live session is released, so a sleeping session holds no
/// VFS in memory. Shell state (working directory and environment) is not kept,
/// so a woken session starts with fresh shell state. See ADR 0006.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/sleep",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "Session is asleep; the snapshot is its wake root", body = TakeSnapshotResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
        (status = 409, description = "The session is already asleep", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn sleep_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<TakeSnapshotResponse>, ApiError> {
    let found = lookup(&state, &owner, &id).await?;
    let session_id = found.state.id;
    let gate = found.gate.clone();
    let _gate = gate.lock().await;

    // Read again under the gate. Another request may have slept or destroyed
    // the session while this one waited.
    let (session, live_state) = {
        let table = state.sessions.0.read().await;
        let entry = table
            .get(&session_id)
            .ok_or_else(|| SandboxError::not_found(format!("session {id} not found")))?;
        let session = entry.session.clone().ok_or_else(asleep)?;
        (session, entry.state.clone())
    };

    let _admit = admit_snapshot(&state, &owner).await?;
    let checkpoint = session.checkpoint().await?;
    let root = checkpoint.root;
    let snapshot_id = store_snapshot(&state, owner, &live_state, root).await;

    let mut table = state.sessions.0.write().await;
    if let Some(entry) = table.get_mut(&session_id) {
        // Dropping the live session here releases its VFS once no other handle
        // to it remains.
        entry.session = None;
        entry.checkpoint = Some(checkpoint);
        entry.state.status = SessionStatus::Sleeping;
        entry.state.updated_at = SystemTime::now();
    }
    drop(table);
    persist(&state).await?;
    Ok(Json(TakeSnapshotResponse { snapshot_id }))
}

/// Wake a sleeping session. The filesystem is restored from the root that
/// sleep recorded. The session keeps its ID.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/wake",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "Session is running again", body = SessionState),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
        (status = 409, description = "The session is not asleep", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn wake_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, ApiError> {
    let not_found = || SandboxError::not_found(format!("session {id} not found"));
    let session_id: SessionId = id.parse().map_err(|_| not_found())?;
    // The write lock covers the check and the restore, so two wakes cannot both
    // succeed.
    let mut table = state.sessions.0.write().await;
    let entry = match table.get_mut(&session_id) {
        Some(entry) if entry.owner == owner => entry,
        _ => return Err(not_found().into()),
    };
    if entry.session.is_some() {
        return Err(SandboxError::new(ErrorCode::InvalidState, "the session is not asleep").into());
    }
    let checkpoint = entry
        .checkpoint
        .clone()
        .ok_or_else(|| SandboxError::internal("a sleeping session has no checkpoint"))?;
    let session: Arc<dyn Session> = restore_checkpoint(checkpoint).await?.into();
    entry.session = Some(session);
    entry.state.status = SessionStatus::Running;
    entry.state.updated_at = SystemTime::now();
    let woken = entry.state.clone();
    drop(table);
    persist(&state).await?;
    Ok(Json(woken))
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
        let mut entry = entry(
            owner.clone(),
            session.clone(),
            TenantId::nil(),
            HashMap::new(),
        );
        stamp(&state, &mut entry, &session).await?;
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
