//! REST handlers for `/v1`. Sessions belong to the API key that created them. A
//! key that did not create a session sees it as not found.

use crate::auth::{AuthManager, KeyId};
use crate::persist::{CheckpointRoot, SpilledRun};
use axum::extract::ws::{Message, WebSocket};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, Request, State, WebSocketUpgrade},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Extension, Router,
};
use sandbox_scheduler::Scheduler;
use sandbox_session::{
    cap_output, changed_since, committed_root, delete_file as delete_session_file,
    restore_checkpoint_limited, vfs_error, write_file as write_session_file, Checkpoint,
    IsolateBackend, OutputChunk, OutputObserver, Session, DEFAULT_OUTPUT_LIMIT,
};
use sandbox_trace::{outcome_of, FileChange, FsOp, RunRecord, Trace};
use sandbox_types::hash::ContentHash;
use sandbox_types::policy::{NetworkPolicy, PolicyAction, ResourceLimits};
use sandbox_types::session::DEFAULT_CHECKPOINT_RETAIN;
use sandbox_types::{
    CheckpointPolicy, ErrorCode, ExecEvent, ExecRequest, ExecResult, OutputStream, SandboxError,
    SessionConfig, SessionId, SessionState, SessionStatus, Snapshot, SnapshotId, TenantId, Tier,
    TierPreference,
};
use sandbox_vfs::OpenFlags;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

/// What one key has used. Counts since the server started, and held in memory,
/// so a restart resets them. They are not billing records (PLAN §4.14).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct Counters {
    pub sessions_created: u64,
    pub sessions_slept: u64,
    pub sessions_woken: u64,
    pub execs: u64,
    /// Wall time of the commands the key ran, in milliseconds.
    pub exec_wall_ms: u64,
    /// Bytes written through the filesystem API.
    pub bytes_written: u64,
}

/// Usage counters for every key, keyed by the key's ID.
#[derive(Clone, Default)]
pub(crate) struct UsageTable(Arc<std::sync::Mutex<HashMap<String, Counters>>>);

impl UsageTable {
    /// Applies `change` to the owner's counters.
    fn add(&self, owner: &KeyId, change: impl FnOnce(&mut Counters)) {
        let mut table = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(table.entry(owner.as_str().to_string()).or_default());
    }

    fn get(&self, owner: &KeyId) -> Counters {
        let table = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        table.get(owner.as_str()).cloned().unwrap_or_default()
    }

    /// The counters of every key added together.
    fn total(&self) -> Counters {
        let table = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        table.values().fold(Counters::default(), |mut sum, c| {
            sum.sessions_created += c.sessions_created;
            sum.sessions_slept += c.sessions_slept;
            sum.sessions_woken += c.sessions_woken;
            sum.execs += c.execs;
            sum.exec_wall_ms += c.exec_wall_ms;
            sum.bytes_written += c.bytes_written;
            sum
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub auth: Arc<AuthManager>,
    pub sessions: SessionTable,
    pub snapshots: SnapshotTable,
    /// Where keys, snapshots, and sleeping sessions are saved. `None` keeps the
    /// server in memory (ADR 0008).
    pub store: Option<Arc<crate::persist::Persistence>>,
    /// When the last garbage collection started. Limits how often it can run.
    pub(crate) gc_last_run: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
    /// What each key has used since this server started (PLAN §4.14).
    pub(crate) usage: UsageTable,
    /// The Tier 2 guest the server can promote a session into. `None` when no image is
    /// configured, and then a command the isolate tier cannot run is refused.
    pub(crate) vm: Option<Arc<VmConfig>>,
}

/// A snapshot taken through the API. Only the key that took it can see it, and
/// the root is the filesystem state it restores.
#[derive(Clone)]
pub(crate) struct SnapshotRecord {
    owner: KeyId,
    session: SessionId,
    pub(crate) root: ContentHash,
    created_at: SystemTime,
    labels: HashMap<String, String>,
    /// The limits of the session it was taken from. A fork gets them (ADR 0014).
    pub(crate) limits: ResourceLimits,
    /// Whether the session it was taken from could move to a VM. A fork inherits it.
    pub(crate) allow_vm: bool,
}

/// Snapshots held by the server, keyed by ID. The server owns this table, so
/// the snapshot limits and the owner checks live in one place.
#[derive(Clone, Default)]
pub struct SnapshotTable {
    pub(crate) records: Arc<RwLock<HashMap<SnapshotId, SnapshotRecord>>>,
    /// Serializes snapshot admission, so the limits cannot be raced past.
    admit: Arc<tokio::sync::Mutex<()>>,
}

/// Most sessions one fork call may create.
const MAX_FORK_COUNT: u32 = 64;
/// Live sessions one key may hold. Sessions use process memory, so this bounds
/// what a single key can take from the server.
pub const MAX_SESSIONS_PER_KEY: usize = 64;
/// Where the server finds a Tier 2 guest: QEMU, and the image directory that holds
/// `vmlinuz` and `initrd.gz` (see docker/emulated-vm/build-image.sh).
#[derive(Clone, Debug)]
pub struct VmConfig {
    pub qemu: std::path::PathBuf,
    pub image: std::path::PathBuf,
    pub accel: sandbox_vmm::Accel,
    pub memory_mib: u32,
    pub boot_timeout: std::time::Duration,
}

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
pub struct SessionTable(pub(crate) Arc<RwLock<HashMap<SessionId, Entry>>>);

pub(crate) struct Entry {
    owner: KeyId,
    state: SessionState,
    /// `None` while the session sleeps. Sleep drops the live session, so a
    /// sleeping session holds no VFS in memory. See ADR 0006.
    pub(crate) session: Option<Arc<dyn Session>>,
    /// The session's last checkpoint: what a restart or a wake brings back. Only
    /// a durable server keeps one (ADR 0010).
    pub(crate) checkpoint: Option<Checkpoint>,
    /// Commands the session ran, in order (ADR 0009).
    trace: Vec<RunRecord>,
    /// Bytes the trace holds in its scripts and outputs.
    trace_bytes: usize,
    /// False once a command could not be recorded faithfully, or the trace is full.
    trace_complete: bool,
    /// Held for every command, file operation, and snapshot on this session.
    /// A checkpoint must not interleave with a write, or the write can be lost
    /// from the snapshot or from the overlay.
    pub(crate) gate: Arc<tokio::sync::Mutex<()>>,
    /// The session's limits, set when it was created (ADR 0014). A wake, a
    /// snapshot, and a restart all keep them.
    pub(crate) limits: ResourceLimits,
    /// When a request last touched the session, in milliseconds since the Unix
    /// epoch. Idle sleep reads it (PLAN §4.14).
    pub(crate) last_active: Arc<std::sync::atomic::AtomicU64>,
    /// When the session checkpoints (IMAGE.md §4.5).
    pub(crate) checkpoint_policy: CheckpointPolicy,
    /// How many checkpoint roots are pinned. Older roots are unpinned.
    pub(crate) checkpoint_retain: u32,
    /// Pinned checkpoint roots, oldest first. The last one is the newest checkpoint.
    pub(crate) roots: Vec<CheckpointRoot>,
    /// The sequence number of the session's next recorded or spilled run.
    pub(crate) next_seq: u64,
    /// Outputs of runs that did not fit in the trace, kept as blobs.
    pub(crate) spilled: Vec<SpilledRun>,
    /// Bytes of the spilled outputs, counted against `spilled_budget_per_session()`.
    pub(crate) spilled_bytes: u64,
}

impl Entry {
    /// Records that a request touched the session now.
    fn touch(&self) {
        self.last_active
            .store(now_millis(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Milliseconds since the Unix epoch.
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
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
#[derive(Debug)]
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
            ErrorCode::Unavailable | ErrorCode::NeedsVm => StatusCode::SERVICE_UNAVAILABLE,
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
    // Memory limit is enforced via VFS quotas and cgroups (worker tier).
    // CPU, process, and open file limits are now enforced in the isolate backend.
    // Wall timeout is enforced at the session level (default 300s).
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
    limits: ResourceLimits,
    allow_vm: bool,
) -> Entry {
    let now = SystemTime::now();
    let state = SessionState {
        id: session.id(),
        tenant_id,
        tier: Tier::Isolate,
        allow_vm,
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
        limits,
        last_active: Arc::new(std::sync::atomic::AtomicU64::new(now_millis())),
        checkpoint_policy: CheckpointPolicy::default(),
        checkpoint_retain: DEFAULT_CHECKPOINT_RETAIN,
        roots: Vec::new(),
        next_seq: 1,
        spilled: Vec::new(),
        spilled_bytes: 0,
    }
}

/// A session the caller owns. A session owned by another key is not found, so
/// the response does not reveal that it exists.
/// A session the caller owns. `session` is `None` while it sleeps.
struct Lookup {
    session: Option<Arc<dyn Session>>,
    state: SessionState,
    gate: Arc<tokio::sync::Mutex<()>>,
    limits: ResourceLimits,
}

/// Whether a session may move to a VM. The state says so, and the server must have an
/// image to move it to.
fn promotable(state: &AppState, owned_state: &SessionState) -> bool {
    owned_state.allow_vm && state.vm.is_some()
}

/// A live session the caller owns.
struct Owned {
    owner: KeyId,
    session: Arc<dyn Session>,
    state: SessionState,
    gate: Arc<tokio::sync::Mutex<()>>,
    limits: ResourceLimits,
}

/// Finds a session the caller owns, sleeping or not. Other keys get the same
/// not-found answer as a missing session.
async fn lookup(state: &AppState, owner: &KeyId, raw: &str) -> Result<Lookup, ApiError> {
    let not_found = || SandboxError::not_found(format!("session {raw} not found"));
    let id: SessionId = raw.parse().map_err(|_| not_found())?;
    let table = state.sessions.0.read().await;
    match table.get(&id) {
        Some(entry) if entry.owner == *owner => {
            entry.touch();
            Ok(Lookup {
                session: entry.session.clone(),
                state: entry.state.clone(),
                gate: entry.gate.clone(),
                limits: entry.limits.clone(),
            })
        }
        _ => Err(not_found().into()),
    }
}

/// Finds a live session the caller owns. A sleeping session is refused until
/// it is woken.
async fn owned(state: &AppState, owner: &KeyId, raw: &str) -> Result<Owned, ApiError> {
    let mut found = lookup(state, owner, raw).await?;
    // A request to a sleeping session wakes it first (PLAN §4.14).
    if found.session.is_none() {
        wake_entry(state, found.state.id, None).await?;
        found = lookup(state, owner, raw).await?;
    }
    match found.session {
        Some(session) => Ok(Owned {
            owner: owner.clone(),
            session,
            state: found.state,
            gate: found.gate,
            limits: found.limits,
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
    create_router_with_state(scheduler, auth).0
}

/// Like [`create_router`], and also returns the server state, so a caller can
/// schedule garbage collection with [`crate::gc::schedule`].
pub fn create_router_with_state(
    scheduler: Arc<Scheduler>,
    auth: Arc<AuthManager>,
) -> (Router, AppState) {
    router_with_vm(scheduler, auth, None)
}

/// Like [`create_router_with_state`], and sessions may move to the Tier 2 guest `vm`
/// describes when a command needs one.
pub fn create_router_with_vm(
    scheduler: Arc<Scheduler>,
    auth: Arc<AuthManager>,
    vm: VmConfig,
) -> (Router, AppState) {
    router_with_vm(scheduler, auth, Some(Arc::new(vm)))
}

fn router_with_vm(
    scheduler: Arc<Scheduler>,
    auth: Arc<AuthManager>,
    vm: Option<Arc<VmConfig>>,
) -> (Router, AppState) {
    let state = AppState {
        scheduler,
        auth,
        sessions: SessionTable::default(),
        snapshots: SnapshotTable::default(),
        gc_last_run: Arc::new(tokio::sync::Mutex::new(None)),
        usage: UsageTable::default(),
        store: None,
        vm,
    };
    (build_router(state.clone()), state)
}

/// Opens a durable server whose state lives in `dir` (ADR 0008). Keys, snapshots,
/// and sleeping sessions from an earlier run are loaded. On the first run one key
/// is created, and it is returned so the caller can show it once.
pub async fn open_durable(
    scheduler: Arc<Scheduler>,
    dir: &std::path::Path,
) -> Result<(Router, Option<String>), SandboxError> {
    let (app, _, new_key) = open_durable_with_state(scheduler, dir).await?;
    Ok((app, new_key))
}

/// Like [`open_durable`], and also returns the server state (see
/// [`create_router_with_state`]).
pub async fn open_durable_with_state(
    scheduler: Arc<Scheduler>,
    dir: &std::path::Path,
) -> Result<(Router, AppState, Option<String>), SandboxError> {
    open_durable_with_vm(scheduler, dir, None).await
}

/// Like [`open_durable_with_state`], and sessions may move to the Tier 2 guest `vm`
/// describes when a command needs one.
pub async fn open_durable_with_vm(
    scheduler: Arc<Scheduler>,
    dir: &std::path::Path,
    vm: Option<VmConfig>,
) -> Result<(Router, AppState, Option<String>), SandboxError> {
    let (store, saved) = crate::persist::Store::open(dir)?;
    attach_store(
        scheduler,
        crate::persist::Persistence::Files(store),
        saved,
        vm.map(Arc::new),
    )
    .await
}

/// Like [`open_durable_with_state`], with the metadata in the Postgres database at
/// `database_url`, so several nodes can serve the same state. The blobs stay in
/// `dir`, which every node must be able to reach.
#[cfg(feature = "clustered")]
pub async fn open_clustered_with_state(
    scheduler: Arc<Scheduler>,
    database_url: &str,
    dir: &std::path::Path,
) -> Result<(Router, AppState, Option<String>), SandboxError> {
    crate::persist::prepare_blobs(dir)?;
    let (store, saved) = crate::pg_store::PgStore::connect(database_url).await?;
    attach_store(
        scheduler,
        crate::persist::Persistence::Clustered(store),
        saved,
        None,
    )
    .await
}

/// Builds the server around a durable store and its saved state: restores the
/// keys, snapshots, and sessions, then saves the restored state.
async fn attach_store(
    scheduler: Arc<Scheduler>,
    store: crate::persist::Persistence,
    saved: crate::persist::Saved,
    vm: Option<Arc<VmConfig>>,
) -> Result<(Router, AppState, Option<String>), SandboxError> {
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
        gc_last_run: Arc::new(tokio::sync::Mutex::new(None)),
        usage: UsageTable::default(),
        vm,
        store: Some(Arc::new(store)),
    };
    restore(&state, saved).await?;
    persist(&state).await.map_err(|e| e.0)?;
    Ok((build_router(state.clone()), state, new_key))
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
                    limits: s.limits,
                    // Stored snapshots predate the flag, so they are not promotable.
                    allow_vm: false,
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
            limits: ResourceLimits::default(),
            trace: s.trace,
            trace_complete: s.trace_complete,
            checkpoint_policy: CheckpointPolicy::default(),
            checkpoint_retain: DEFAULT_CHECKPOINT_RETAIN,
            roots: Vec::new(),
            spilled: Vec::new(),
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
            Some(
                restore_checkpoint_limited(checkpoint.clone(), &saved_session.limits)
                    .await?
                    .into(),
            )
        };
        let id = saved_session.state.id;
        let next_seq = next_seq_after(&trace.runs, &saved_session.spilled);
        let spilled_bytes = saved_session.spilled.iter().map(|s| s.bytes).sum();
        let spilled = saved_session.spilled;
        // Saved state is not trusted to respect the retention bounds, so clamp them.
        let retain = saved_session
            .checkpoint_retain
            .clamp(1, MAX_CHECKPOINT_RETAIN);
        let mut roots = saved_session.roots;
        let excess = roots.len().saturating_sub(retain as usize);
        roots.drain(..excess);
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
                limits: saved_session.limits,
                checkpoint_policy: saved_session.checkpoint_policy,
                checkpoint_retain: retain,
                roots,
                next_seq,
                spilled,
                spilled_bytes,
                last_active: Arc::new(std::sync::atomic::AtomicU64::new(now_millis())),
            },
        );
    }
    Ok(())
}

/// The durable state as it is now.
async fn capture(state: &AppState) -> Result<crate::persist::Saved, SandboxError> {
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
                limits: r.limits.clone(),
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
                let saved = Trace {
                    runs: e.trace.clone(),
                }
                .to_jsonl()
                .map(|trace| SavedSession {
                    owner: e.owner.as_str().to_string(),
                    state: e.state.clone(),
                    root: checkpoint.root,
                    cwd: checkpoint.cwd.clone(),
                    // Environment values are often secrets, so they are never written
                    // to disk. A restored session starts with an empty environment (ADR 0010).
                    env: HashMap::new(),
                    sleeping: e.session.is_none(),
                    limits: e.limits.clone(),
                    trace,
                    trace_complete: e.trace_complete,
                    checkpoint_policy: e.checkpoint_policy,
                    checkpoint_retain: e.checkpoint_retain,
                    roots: e.roots.clone(),
                    spilled: e.spilled.clone(),
                })
                .map_err(|err| {
                    SandboxError::internal(format!("cannot encode the trace of a session: {err}"))
                });
                Some(saved)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(Saved {
        version: crate::persist::VERSION,
        keys,
        snapshots,
        sleeping: Vec::new(),
        sessions,
        // The store sets the epoch when it writes the base file.
        log_epoch: 0,
    })
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

/// The caller's usage counters since this server started.
#[utoipa::path(
    get,
    path = "/v1/usage",
    tag = "usage",
    responses(
        (status = 200, description = "The caller's counters", body = Counters),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn get_usage(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
) -> Json<Counters> {
    Json(state.usage.get(&owner))
}

/// Server totals in the Prometheus text format (version 0.0.4). The counters sum every
/// key, so no key or tenant appears in the output. The gauges read the current tables.
/// The route requires an API key, so the totals are not public.
pub async fn get_metrics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let total = state.usage.total();
    let sessions = state.sessions.0.read().await.len();
    let snapshots = state.snapshots.records.read().await.len();
    let counters = [
        (
            "sandbox_sessions_created_total",
            "Sessions created since start.",
            total.sessions_created,
        ),
        (
            "sandbox_sessions_slept_total",
            "Sessions put to sleep since start.",
            total.sessions_slept,
        ),
        (
            "sandbox_sessions_woken_total",
            "Sessions woken since start.",
            total.sessions_woken,
        ),
        (
            "sandbox_execs_total",
            "Commands run since start.",
            total.execs,
        ),
        (
            "sandbox_exec_wall_ms_total",
            "Wall time of commands run, in milliseconds.",
            total.exec_wall_ms,
        ),
        (
            "sandbox_bytes_written_total",
            "Bytes written through the filesystem API.",
            total.bytes_written,
        ),
    ];
    let gauges = [
        (
            "sandbox_sessions_active",
            "Sessions the server holds now.",
            sessions as u64,
        ),
        (
            "sandbox_snapshots_active",
            "Snapshots the server holds now.",
            snapshots as u64,
        ),
    ];
    let mut body = String::new();
    for (kind, metrics) in [("counter", &counters[..]), ("gauge", &gauges[..])] {
        for (name, help, value) in metrics {
            body.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}\n"
            ));
        }
    }
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id", delete(destroy_session))
        .route("/v1/sessions/:id/exec", post(exec_command))
        .route("/v1/sessions/:id/ws", get(exec_websocket))
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
        .route("/v1/sessions/:id/info", get(session_info))
        .route("/v1/sessions/:id/checkpoint", post(checkpoint_session))
        .route("/v1/sessions/:id/undo", post(undo_session))
        .route("/v1/sessions/:id/fork", post(fork_command))
        .route("/v1/gc", post(crate::gc::run_gc))
        .route("/v1/usage", get(get_usage))
        .route("/metrics", get(get_metrics))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .route("/v1/openapi.json", get(openapi_json))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(crate::middleware::request_context))
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
    if config.checkpoint_retain == 0 || config.checkpoint_retain > MAX_CHECKPOINT_RETAIN {
        return Err(SandboxError::invalid_argument(format!(
            "checkpoint_retain must be between 1 and {MAX_CHECKPOINT_RETAIN}"
        ))
        .into());
    }
    let tenant_id = config.tenant_id;
    let labels = config.labels.clone();
    let limits = config.limits.clone();
    let network = config.network.clone();
    let session: Arc<dyn Session> = match config.initial_snapshot {
        Some(snapshot_id) => {
            let record = snapshot_for(&state, &owner, snapshot_id).await?;
            sandbox_session::restore_limited(record.root, &limits)
                .await?
                .into()
        }
        None => Arc::new(IsolateBackend::new(config.clone()).await?),
    };
    let id = session.id();
    let mut entry = entry(
        owner.clone(),
        session.clone(),
        tenant_id,
        labels,
        limits.clone(),
        config.tier_preference != TierPreference::Isolate,
    );
    entry.checkpoint_policy = config.checkpoint;
    entry.checkpoint_retain = config.checkpoint_retain;
    stamp(&state, &mut entry, &session).await?;
    write_env_json(&session, &env_json(id, session.tier(), &limits, &network)).await?;
    admit_session(&state, &owner, entry).await?;
    state.usage.add(&owner, |c| c.sessions_created += 1);
    checkpoint_now(&state, &owner, id, &session).await?;
    // A new session is saved now, so a restart before its first change keeps it (ADR 0010).
    persist(&state).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateSessionResponse { session_id: id }),
    ))
}

/// The contents of `/etc/agent/env.json`, schema 1 (IMAGE.md §4.5). Tier 1 has no
/// image layers or packages yet, so those fields stay empty.
fn env_json(
    id: SessionId,
    tier: Tier,
    limits: &ResourceLimits,
    network: &NetworkPolicy,
) -> serde_json::Value {
    let allow: Vec<&str> = network
        .rules
        .iter()
        .filter(|rule| rule.action == PolicyAction::Allow)
        .map(|rule| rule.host_pattern.as_str())
        .collect();
    serde_json::json!({
        "schema": 1,
        "session": id.to_string(),
        "tier": tier.as_str(),
        "image": {"base": null, "layers": []},
        "packages": [],
        "network": {
            "default": if network.default_deny { "deny" } else { "allow" },
            "allow": allow,
        },
        "limits": {
            "memory_mib": limits.memory_bytes.map(|bytes| bytes / (1024 * 1024)),
            "cpu_ms_budget": null,
            "wall_timeout_s": limits.wall_timeout_secs,
        },
        "gui": {"available": false, "display": null, "width": null, "height": null},
        "commands": ["sandbox"],
        "user": "agent",
    })
}

/// Gives a forked session its own `env.json`. The snapshot's file is copied with the
/// session ID changed, so the network and limits it was created with carry over. A
/// snapshot taken before the file existed gets a fresh one.
async fn write_forked_env_json(
    session: &Arc<dyn Session>,
    id: SessionId,
    limits: &ResourceLimits,
) -> Result<(), SandboxError> {
    let inherited = read_session_file(session.as_ref(), sandbox_session::ENV_JSON_PATH).await?;
    // A copy over the host-file cap, or one that does not parse, is not trusted. The
    // child gets a fresh file instead.
    let inherited =
        inherited.filter(|bytes| bytes.len() as u64 <= sandbox_vfs::SYSTEM_FILE_MAX_BYTES);
    let env = match inherited
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    {
        Some(mut env) => {
            env["session"] = serde_json::json!(id.to_string());
            env
        }
        None => env_json(id, Tier::Isolate, limits, &NetworkPolicy::default()),
    };
    write_env_json(session, &env).await
}

/// Reads a whole file from the session. `Ok(None)` means the file does not exist.
async fn read_session_file(
    session: &dyn Session,
    path: &str,
) -> Result<Option<Vec<u8>>, SandboxError> {
    let fs = session.fs();
    let target = std::path::Path::new(path);
    let stat = match fs.stat(target).await {
        Ok(stat) => stat,
        Err(sandbox_vfs::VfsError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(vfs_error(e)),
    };
    let handle = fs.open(target, read_flags()).await.map_err(vfs_error)?;
    let read = fs.read(&handle, 0, stat.size as usize).await;
    fs.close(&handle);
    Ok(Some(read.map_err(vfs_error)?.to_vec()))
}

/// Writes `/etc/agent/env.json` into the session. A session without it breaks the
/// I0 acceptance rule, so a failure here fails the request.
async fn write_env_json(
    session: &Arc<dyn Session>,
    env: &serde_json::Value,
) -> Result<(), SandboxError> {
    let data = serde_json::to_vec_pretty(env)
        .map_err(|e| SandboxError::internal(format!("env.json encode: {e}")))?;
    for dir in ["/etc", "/etc/agent"] {
        match session.fs().mkdir(std::path::Path::new(dir), 0o755).await {
            Ok(()) | Err(sandbox_vfs::VfsError::AlreadyExists(_)) => {}
            Err(e) => return Err(sandbox_session::vfs_error(e)),
        }
    }
    session
        .write_system_file(sandbox_session::ENV_JSON_PATH, &data)
        .await
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
) -> Result<Json<SessionView>, ApiError> {
    let session = lookup(&state, &owner, &id).await?.state;
    let checkpoints = checkpoint_usage_of(&state, session.id).await;
    Ok(Json(SessionView {
        session,
        checkpoints,
    }))
}

/// A session as GET returns it: its state, and what its checkpoints hold.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionView {
    #[serde(flatten)]
    pub session: SessionState,
    pub checkpoints: CheckpointUsage,
}

/// The checkpoints a session has pinned (IMAGE.md §4.5).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CheckpointUsage {
    pub policy: CheckpointPolicy,
    /// Checkpoints pinned now.
    pub count: u32,
    /// The most that are pinned. Older checkpoints are unpinned.
    pub retain: u32,
    /// Logical bytes of the pinned roots. A block shared by two roots counts twice.
    /// Unpinned blocks are not freed until GC lands.
    pub bytes: u64,
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

/// Run one command in a session. With `stream`, the output comes as NDJSON
/// `ExecEvent` lines while the command runs (PLAN §4.15).
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/exec",
    tag = "exec",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ExecRequest,
    responses(
        (status = 200, description = "Command finished. Buffered, the whole result. Streamed (`?stream=true`), `application/x-ndjson` lines, each an `ExecEvent`", body = ExecResult),
        (status = 400, description = "Invalid request, such as an empty argv", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn exec_command(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Query(query): Query<ExecQuery>,
    Json(request): Json<ExecRequest>,
) -> Result<Response, ApiError> {
    if query.stream.unwrap_or(false) || request.stream {
        return exec_streaming(state, owner, id, request).await;
    }
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    if let Some(result) = host_sandbox_command(&state, &own, &request.argv).await {
        return Ok(Json(result).into_response());
    }
    let limit = request.output_limit.map_or(DEFAULT_OUTPUT_LIMIT, |limit| {
        usize::try_from(limit).unwrap_or(usize::MAX)
    });
    // A command the isolate tier cannot run moves the session to a VM, and runs there. The
    // gate is still held, so no other command interleaves with the move.
    let (mut result, seq) = match run_exec(&state, &owner, &own, request.clone(), None).await {
        Err(error) if error.0.code == ErrorCode::NeedsVm && promotable(&state, &own.state) => {
            let promoted = promote_session(&state, &owner, &own).await?;
            run_exec(&state, &owner, &promoted, request, None).await?
        }
        outcome => outcome?,
    };
    cap_result(&mut result, limit, seq);
    Ok(Json(result).into_response())
}

/// Moves a session to a Tier 2 VM. The guest starts, the session's files are copied in, and
/// the table entry takes the VM session, under the same ID. Its trace and its gate carry over.
/// Shell state and running processes do not: they stay on the isolate session.
async fn promote_session(state: &AppState, owner: &KeyId, own: &Owned) -> Result<Owned, ApiError> {
    let Some(config) = state.vm.clone() else {
        return Err(SandboxError::new(
            ErrorCode::NeedsVm,
            "this server has no VM image, so the command cannot run",
        )
        .into());
    };
    let spec = sandbox_vmm::QemuSpec {
        qemu: config.qemu.clone(),
        kernel: config.image.join("vmlinuz"),
        initrd: config.image.join("initrd.gz"),
        memory_mib: config.memory_mib,
        accel: config.accel,
        console: config.image.join(format!("console-{}.log", own.state.id)),
    };
    let mut vm = sandbox_vmm::QemuVm::start(&spec).await?;
    vm.wait_for_agent(config.boot_timeout).await?;
    let (promoted, _report) = sandbox_promote::promote_to_vm(own.session.as_ref(), vm).await?;
    {
        let mut table = state.sessions.0.write().await;
        let Some(entry) = table.get_mut(&own.state.id) else {
            return Err(SandboxError::not_found("the session is gone").into());
        };
        entry.session = Some(Arc::new(promoted));
        entry.state.tier = Tier::Vm;
    }
    owned(state, owner, &own.state.id.to_string()).await
}

/// Caps both streams of a buffered result (IMAGE.md §4.5). The full streams are the
/// trace's `/trace/exec/<seq>.out` and `.err`. `seq` is `None` when the run was not
/// recorded, so the markers say the output is not available.
fn cap_result(result: &mut ExecResult, limit: usize, seq: Option<u64>) {
    let out_path = seq.map(|seq| format!("/trace/exec/{seq}.out"));
    let err_path = seq.map(|seq| format!("/trace/exec/{seq}.err"));
    result.stdout = cap_output(&result.stdout, limit, out_path.as_deref());
    result.stderr = cap_output(&result.stderr, limit, err_path.as_deref());
}

/// The NDJSON line for one chunk of a command's output.
fn output_event(chunk: OutputChunk) -> ExecEvent {
    let stream = if chunk.fd == 2 {
        OutputStream::Stderr
    } else {
        OutputStream::Stdout
    };
    ExecEvent::Output {
        stream,
        data: chunk.data,
    }
}

/// Runs a command for a request, and does what every command does afterwards: it
/// is metered, recorded in the trace, and the session is saved. The caller holds
/// the session's gate.
async fn run_exec(
    state: &AppState,
    owner: &KeyId,
    own: &Owned,
    request: ExecRequest,
    observer: Option<OutputObserver>,
) -> Result<(ExecResult, Option<u64>), ApiError> {
    let session = own.session.clone();
    let before = committed_root(session.as_ref()).await;
    // The generator state at the start of the run, so a replay draws the same $RANDOM values.
    let random_state = Some(session.random_state().await);
    let result = match observer {
        Some(observer) => session.exec_streamed(request.clone(), observer).await,
        None => session.exec(request.clone()).await,
    };
    state.usage.add(owner, |c| {
        c.execs += 1;
        if let Ok(out) = &result {
            c.exec_wall_ms += out.wall_ms;
        }
    });
    let files = files_since(session.as_ref(), before).await;
    let seq = record_exec(state, own.state.id, &request, &result, files, random_state).await;
    checkpoint_after_exec(state, owner, own.state.id, &session).await?;
    save_session(state, own.state.id, &session).await?;
    Ok((result?, seq))
}

/// Runs a command and streams its output as NDJSON. The response starts before
/// the command finishes, so a failure after that point is an `error` line, not an
/// HTTP status. A failure before it, such as an unknown session, is an HTTP error.
/// The command keeps running if the client disconnects, so the session still ends
/// in a consistent state, and its trace still records the run.
async fn exec_streaming(
    state: AppState,
    owner: KeyId,
    id: String,
    request: ExecRequest,
) -> Result<Response, ApiError> {
    if request.argv.is_empty() {
        return Err(SandboxError::invalid_argument("argv is empty").into());
    }
    let own = owned(&state, &owner, &id).await?;
    let gate = own.gate.clone().lock_owned().await;
    let lines = start_exec(state, owner, own, request, gate, CancellationToken::new());
    let body = UnboundedReceiverStream::new(lines).map(|event| {
        // An event that cannot be encoded ends the stream with an error, so the
        // client sees a truncated response rather than a silent gap.
        let mut line = serde_json::to_vec(&event).map_err(std::io::Error::other)?;
        line.push(b'\n');
        Ok::<Bytes, std::io::Error>(Bytes::from(line))
    });
    let mut response = Response::new(Body::from_stream(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}

/// Runs `request` in the background and returns the stream of its events. The stream
/// ends with an `Exit`, an `Error`, or, when `cancel` fires first, a `CANCELLED` error.
/// The command keeps `gate` until it ends, so no other command or checkpoint runs on
/// the session meanwhile. A cancelled run is dropped at its next await, as a timed-out
/// one is, so its partial writes stay in the filesystem.
fn start_exec(
    state: AppState,
    owner: KeyId,
    own: Owned,
    request: ExecRequest,
    gate: tokio::sync::OwnedMutexGuard<()>,
    cancel: CancellationToken,
) -> UnboundedReceiver<ExecEvent> {
    let (events, lines) = tokio::sync::mpsc::unbounded_channel::<ExecEvent>();
    let output = events.clone();
    let observer: OutputObserver = Arc::new(move |chunk: OutputChunk| {
        // A send fails only after the client has gone away. The command still runs.
        let _ = output.send(output_event(chunk));
    });
    tokio::spawn(async move {
        let _gate = gate;
        let last = tokio::select! {
            outcome = run_exec(&state, &owner, &own, request, Some(observer)) => match outcome {
                Ok((result, _)) => ExecEvent::Exit {
                    exit_code: result.exit_code,
                    cpu_ms: result.cpu_ms,
                    wall_ms: result.wall_ms,
                },
                Err(ApiError(error)) => ExecEvent::Error {
                    code: error.code,
                    message: error.message,
                },
            },
            () = cancel.cancelled() => ExecEvent::Error {
                code: ErrorCode::Cancelled,
                message: "the command was cancelled".to_string(),
            },
        };
        let _ = events.send(last);
    });
    lines
}

/// The read half of a WebSocket, and the write half.
type SocketRx = futures::stream::SplitStream<WebSocket>;
type SocketTx = futures::stream::SplitSink<WebSocket, Message>;

/// Runs commands over a WebSocket at `/v1/sessions/{id}/ws`. The client's first
/// message is the JSON `ExecRequest`. The server sends each `ExecEvent` as a text
/// message and closes the connection after the final event. While the command runs,
/// the client may send `{"type":"cancel"}` to stop it. Closing the connection also
/// cancels it, because nothing is left to read its output (ADR 0030).
async fn exec_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    Ok(ws.on_upgrade(move |socket: WebSocket| serve_exec_socket(socket, state, owner, own)))
}

async fn serve_exec_socket(socket: WebSocket, state: AppState, owner: KeyId, own: Owned) {
    let (mut tx, mut rx) = futures::StreamExt::split(socket);
    let request = match read_exec_request(&mut rx).await {
        Ok(request) => request,
        Err(error) => {
            send_event(&mut tx, &error_event(error)).await;
            close(&mut tx).await;
            return;
        }
    };
    if request.argv.is_empty() {
        let error = SandboxError::invalid_argument("argv is empty");
        send_event(&mut tx, &error_event(error)).await;
        close(&mut tx).await;
        return;
    }
    let gate = own.gate.clone().lock_owned().await;
    let cancel = CancellationToken::new();
    let mut lines = start_exec(state, owner, own, request, gate, cancel.clone());
    let watcher = tokio::spawn(watch_client(rx, cancel));
    forward_events(&mut tx, &mut lines).await;
    watcher.abort();
    close(&mut tx).await;
}

fn error_event(error: SandboxError) -> ExecEvent {
    ExecEvent::Error {
        code: error.code,
        message: error.message,
    }
}

/// A message the client sends while a command runs.
#[derive(Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    kind: String,
}

/// Watches the client during a command. A cancel message cancels it, and so does the
/// end of the connection, whether a close frame, an error, or the stream ending.
async fn watch_client(mut rx: SocketRx, cancel: CancellationToken) {
    loop {
        match futures::StreamExt::next(&mut rx).await {
            Some(Ok(Message::Text(text)))
                if serde_json::from_str::<ClientMessage>(&text)
                    .is_ok_and(|message| message.kind == "cancel") =>
            {
                return cancel.cancel();
            }
            Some(Ok(
                Message::Text(_) | Message::Binary(_) | Message::Ping(_) | Message::Pong(_),
            )) => {}
            _ => return cancel.cancel(),
        }
    }
}

/// Reads the exec request that must be the connection's first message.
async fn read_exec_request(rx: &mut SocketRx) -> Result<ExecRequest, SandboxError> {
    match futures::StreamExt::next(rx).await {
        Some(Ok(Message::Text(text))) => serde_json::from_str(&text).map_err(|e| {
            SandboxError::invalid_argument(format!("the first message is not an exec request: {e}"))
        }),
        _ => Err(SandboxError::invalid_argument(
            "the first message must be a JSON exec request",
        )),
    }
}

/// Sends one event as a text message. False when the event cannot be sent or the
/// client has gone away.
async fn send_event(tx: &mut SocketTx, event: &ExecEvent) -> bool {
    let Ok(json) = serde_json::to_string(event) else {
        return false;
    };
    futures::SinkExt::send(tx, Message::Text(json))
        .await
        .is_ok()
}

/// A close frame, so the client can tell that the stream is complete rather than dropped.
async fn close(tx: &mut SocketTx) {
    let _ = futures::SinkExt::send(tx, Message::Close(None)).await;
}

/// Sends each event as a text message until the command ends or the client leaves.
async fn forward_events(tx: &mut SocketTx, lines: &mut UnboundedReceiver<ExecEvent>) {
    while let Some(event) = lines.recv().await {
        if !send_event(tx, &event).await {
            return;
        }
    }
}

/// Most runs one session's trace holds. Runs past this are not recorded, and the
/// trace reports itself incomplete, so a long session never fails over its trace.
pub const MAX_TRACE_RUNS: usize = 4096;
/// Most bytes of scripts and output one session's trace holds. Output is the
/// bulk of a trace, and a session's trace is kept in memory and saved with the
/// state, so the budget bounds both.
pub const MAX_TRACE_BYTES: usize = 8 * 1024 * 1024;

/// Bytes a run takes in a trace: its script, its output, and the paths it changed.
fn run_size(run: &RunRecord) -> usize {
    let outcome = match &run.outcome {
        sandbox_trace::Outcome::Completed { stdout, stderr, .. } => stdout.len() + stderr.len(),
        sandbox_trace::Outcome::Failed { message, .. } => message.len(),
    };
    let files: usize = run.files.iter().map(|f| f.path.len() + 64).sum();
    run.script.len() + outcome + files
}

fn trace_size(runs: &[RunRecord]) -> usize {
    runs.iter().map(run_size).sum()
}

/// One run as the trace records it, before it is appended.
struct Ran {
    /// The script, or `None` when the run cannot be replayed faithfully. The
    /// trace is then marked incomplete. A filesystem run has an empty script.
    script: Option<String>,
    /// The filesystem write or delete, when the run is one.
    fs: Option<FsOp>,
    result: Result<(i32, Vec<u8>, Vec<u8>), SandboxError>,
    /// The files the run changed, or the error that stopped them being read.
    files: Result<Vec<FileChange>, SandboxError>,
    /// The `$RANDOM` generator state when the run started, for a script run.
    random_state: Option<u64>,
}

/// The most output, in bytes, a session may keep as spilled blobs. Runs past the trace's
/// budget keep their outputs here, so the full stream stays readable.
pub const DEFAULT_SPILLED_BYTES_PER_SESSION: u64 = 64 * 1024 * 1024;

/// The per-session spill budget. `FRONTAL_SANDBOX_SPILLED_BYTES_PER_SESSION` sets it, in
/// bytes, and is read once. An unset or invalid value gives the default.
fn spilled_budget_per_session() -> u64 {
    static BUDGET: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| {
        parse_spill_budget(
            std::env::var("FRONTAL_SANDBOX_SPILLED_BYTES_PER_SESSION")
                .ok()
                .as_deref(),
        )
    })
}

/// Parses a spill budget. A zero or unparseable value falls back to the default.
fn parse_spill_budget(raw: Option<&str>) -> u64 {
    match raw.and_then(|value| value.trim().parse::<u64>().ok()) {
        Some(bytes) if bytes > 0 => bytes,
        _ => DEFAULT_SPILLED_BYTES_PER_SESSION,
    }
}

/// The most spilled runs a session may keep. Each run also costs `SPILL_ENTRY_OVERHEAD`
/// bytes against the budget, so runs with empty output cannot grow the record for free.
pub const MAX_SPILLED_RUNS: usize = 4096;
const SPILL_ENTRY_OVERHEAD: u64 = 256;

/// The sequence number after the last run the session has recorded or spilled.
fn next_seq_after(runs: &[RunRecord], spilled: &[SpilledRun]) -> u64 {
    runs.iter()
        .map(|run| run.seq)
        .chain(spilled.iter().map(|run| run.seq))
        .max()
        .map_or(1, |last| last + 1)
}

/// Appends one run to the session's trace (ADR 0009, ADR 0012). A run that does not fit
/// keeps its outputs as blobs instead (IMAGE.md §4.5), and the trace is marked
/// incomplete. Returns the run's sequence number, or `None` when no output was kept.
async fn append_run(state: &AppState, id: SessionId, ran: Ran) -> Option<u64> {
    let (seq, outcome) = {
        let mut table = state.sessions.0.write().await;
        let entry = table.get_mut(&id)?;
        let seq = entry.next_seq;
        entry.next_seq += 1;
        let outcome = outcome_of(ran.result);
        let files = ran.files.unwrap_or_else(|_| {
            entry.trace_complete = false;
            Vec::new()
        });
        match ran.script {
            Some(script) => {
                let run = RunRecord {
                    seq,
                    script,
                    fs: ran.fs,
                    outcome,
                    files,
                    random_state: ran.random_state,
                };
                let size = run_size(&run);
                if entry.trace.len() < MAX_TRACE_RUNS && entry.trace_bytes + size <= MAX_TRACE_BYTES
                {
                    entry.trace_bytes += size;
                    entry.trace.push(run);
                    return Some(seq);
                }
                entry.trace_complete = false;
                (seq, run.outcome)
            }
            None => {
                // A run that cannot be replayed is not recorded, but its output is kept.
                entry.trace_complete = false;
                (seq, outcome)
            }
        }
    };
    let sandbox_trace::Outcome::Completed { stdout, stderr, .. } = outcome else {
        return None;
    };
    let bytes = (stdout.len() + stderr.len()) as u64 + SPILL_ENTRY_OVERHEAD;
    {
        let table = state.sessions.0.read().await;
        let entry = table.get(&id)?;
        if entry.spilled.len() >= MAX_SPILLED_RUNS
            || entry.spilled_bytes + bytes > spilled_budget_per_session()
        {
            return None;
        }
    }
    let store = sandbox_session::blob_store();
    let stdout_hash = store.put(&stdout, Default::default()).await.ok()?;
    let stderr_hash = store.put(&stderr, Default::default()).await.ok()?;
    let mut table = state.sessions.0.write().await;
    let entry = table.get_mut(&id)?;
    if entry.spilled.len() >= MAX_SPILLED_RUNS
        || entry.spilled_bytes + bytes > spilled_budget_per_session()
    {
        return None;
    }
    entry.spilled_bytes += bytes;
    entry.spilled.push(SpilledRun {
        seq,
        stdout: stdout_hash,
        stderr: stderr_hash,
        bytes,
    });
    Some(seq)
}

/// Appends one command to the session's trace (ADR 0009).
async fn record_exec(
    state: &AppState,
    id: SessionId,
    request: &ExecRequest,
    result: &Result<ExecResult, SandboxError>,
    files: Result<Vec<FileChange>, SandboxError>,
    random_state: Option<u64>,
) -> Option<u64> {
    if request.argv.is_empty() {
        return None;
    }
    // Standard input cannot be written into a script, so the trace is marked
    // incomplete rather than recording a command that replays differently.
    let script = match script_for(request) {
        Some(script) if request.stdin.as_ref().is_none_or(|s| s.is_empty()) => Some(script),
        _ => None,
    };
    let result = result
        .as_ref()
        .map(|out| (out.exit_code, out.stdout.clone(), out.stderr.clone()))
        .map_err(|e| SandboxError::new(e.code, e.message.clone()));
    append_run(
        state,
        id,
        Ran {
            script,
            fs: None,
            result,
            files,
            random_state,
        },
    )
    .await
}

/// Appends one filesystem write or delete to the session's trace (ADR 0012).
/// `op` is `None` when the operation is too large to record, and the trace is
/// then marked incomplete.
async fn record_fs(
    state: &AppState,
    id: SessionId,
    op: Option<FsOp>,
    result: &Result<(), SandboxError>,
    files: Result<Vec<FileChange>, SandboxError>,
) {
    let result = result
        .as_ref()
        .map(|()| (0, Vec::new(), Vec::new()))
        .map_err(|e| SandboxError::new(e.code, e.message.clone()));
    append_run(
        state,
        id,
        Ran {
            script: op.as_ref().map(|_| String::new()),
            fs: op,
            result,
            files,
            random_state: None,
        },
    )
    .await;
}

/// The files changed since `before`, a committed root read before the operation.
async fn files_since(
    session: &dyn Session,
    before: Result<ContentHash, SandboxError>,
) -> Result<Vec<FileChange>, SandboxError> {
    let changes = changed_since(session, before?).await?;
    Ok(changes.into_iter().map(FileChange::from).collect())
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
    let body = Trace { runs }
        .to_jsonl()
        .map_err(|e| SandboxError::internal(format!("cannot encode the trace: {e}")))?;
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
    /// Stream the output as NDJSON `ExecEvent` lines while the command runs.
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
    if let Some((seq, stream)) = trace_output_path(&path) {
        return trace_output(&state, &own.state.id, seq, stream).await;
    }
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

/// The trace output path for a command: `trace/exec/<seq>.out` for stdout and
/// `trace/exec/<seq>.err` for stderr. The path has no leading slash, as in the fs route.
fn trace_output_path(path: &str) -> Option<(u64, OutputStream)> {
    let (seq, ext) = path.strip_prefix("trace/exec/")?.rsplit_once('.')?;
    let stream = match ext {
        "out" => OutputStream::Stdout,
        "err" => OutputStream::Stderr,
        _ => return None,
    };
    Some((seq.parse().ok()?, stream))
}

/// Serves one recorded command's full stdout or stderr from the session's trace.
async fn trace_output(
    state: &AppState,
    id: &SessionId,
    seq: u64,
    stream: OutputStream,
) -> Result<Response, ApiError> {
    // The trace's copy is preferred. A run that did not fit there keeps its output as a blob.
    let stored = {
        let table = state.sessions.0.read().await;
        let entry = table
            .get(id)
            .ok_or_else(|| SandboxError::not_found("session not found"))?;
        if let Some(run) = entry.trace.iter().find(|run| run.seq == seq) {
            let sandbox_trace::Outcome::Completed { stdout, stderr, .. } = &run.outcome else {
                return Err(
                    SandboxError::not_found(format!("command {seq} did not complete")).into(),
                );
            };
            Ok(match stream {
                OutputStream::Stdout => stdout.clone(),
                OutputStream::Stderr => stderr.clone(),
            })
        } else if let Some(spill) = entry.spilled.iter().find(|run| run.seq == seq) {
            Err(match stream {
                OutputStream::Stdout => spill.stdout,
                OutputStream::Stderr => spill.stderr,
            })
        } else {
            return Err(SandboxError::not_found(format!("no recorded command {seq}")).into());
        }
    };
    let bytes = match stored {
        Ok(bytes) => bytes,
        Err(hash) => sandbox_session::blob_store()
            .get(&hash)
            .await
            .map_err(|e| SandboxError::internal(format!("reading spilled output: {e}")))?
            .ok_or_else(|| SandboxError::internal("a spilled output is missing from the store"))?
            .to_vec(),
    };
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
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
    let target = vfs_path(&path)?.display().to_string();
    let before = committed_root(session.as_ref()).await;
    let result = write_session_file(session.as_ref(), &target, &body).await;
    let files = files_since(session.as_ref(), before).await;
    // A write too large for the trace budget is not copied. The trace is marked
    // incomplete instead.
    let op = (body.len() <= MAX_TRACE_BYTES).then(|| FsOp::Write {
        path: target,
        data: body.to_vec(),
    });
    record_fs(&state, own.state.id, op, &result, files).await;
    if result.is_ok() {
        state
            .usage
            .add(&owner, |c| c.bytes_written += body.len() as u64);
    }
    result?;
    save_session(&state, own.state.id, &session).await?;
    Ok(StatusCode::NO_CONTENT)
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
    let target = vfs_path(&path)?.display().to_string();
    let before = committed_root(session.as_ref()).await;
    let result = delete_session_file(session.as_ref(), &target).await;
    let files = files_since(session.as_ref(), before).await;
    record_fs(
        &state,
        own.state.id,
        Some(FsOp::Delete { path: target }),
        &result,
        files,
    )
    .await;
    result?;
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
    let snapshot_id = store_snapshot(&state, owner, &own.state, root, own.limits.clone()).await;
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
    limits: ResourceLimits,
) -> SnapshotId {
    let snapshot_id = SnapshotId::new();
    let record = SnapshotRecord {
        owner,
        session: session.id,
        root,
        created_at: SystemTime::now(),
        labels: session.labels.clone(),
        limits,
        allow_vm: session.allow_vm,
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
    let _gate = found.gate.clone().lock_owned().await;
    let snapshot_id = sleep_locked(&state, session_id).await?;
    Ok(Json(TakeSnapshotResponse { snapshot_id }))
}

/// Puts a session to sleep: snapshots its filesystem, records the checkpoint,
/// and drops the live session. The caller holds the session's gate, so no
/// command can write between the checkpoint and the drop.
pub(crate) async fn sleep_locked(
    state: &AppState,
    session_id: SessionId,
) -> Result<SnapshotId, ApiError> {
    let (owner, session, live_state, limits) = {
        let table = state.sessions.0.read().await;
        let entry = table
            .get(&session_id)
            .ok_or_else(|| SandboxError::not_found(format!("session {session_id} not found")))?;
        let session = entry.session.clone().ok_or_else(asleep)?;
        (
            entry.owner.clone(),
            session,
            entry.state.clone(),
            entry.limits.clone(),
        )
    };

    let _admit = admit_snapshot(state, &owner).await?;
    let checkpoint = session.checkpoint().await?;
    let root = checkpoint.root;
    let snapshot_id = store_snapshot(state, owner.clone(), &live_state, root, limits).await;

    {
        let mut table = state.sessions.0.write().await;
        if let Some(entry) = table.get_mut(&session_id) {
            // Dropping the live session here releases its VFS once no other handle
            // to it remains.
            entry.session = None;
            entry.checkpoint = Some(checkpoint);
            entry.state.status = SessionStatus::Sleeping;
            entry.state.updated_at = SystemTime::now();
        }
    }
    state.usage.add(&owner, |c| c.sessions_slept += 1);
    persist(state).await?;
    Ok(snapshot_id)
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
    let session_id: SessionId = id
        .parse()
        .map_err(|_| SandboxError::not_found(format!("session {id} not found")))?;
    Ok(Json(wake_entry(&state, session_id, Some(&owner)).await?))
}

/// Restores a sleeping session from the checkpoint it was put to sleep with. The
/// table's write lock covers the check and the restore, so two wakes cannot both
/// restore it. With `explicit_owner`, the session must belong to that key and
/// must be asleep, as an explicit wake requires. Without it, an awake session is
/// returned as it is, which is what a request to a sleeping session needs.
pub(crate) async fn wake_entry(
    state: &AppState,
    session_id: SessionId,
    explicit_owner: Option<&KeyId>,
) -> Result<SessionState, ApiError> {
    let mut table = state.sessions.0.write().await;
    let entry = match table.get_mut(&session_id) {
        Some(entry) if explicit_owner.is_none_or(|owner| entry.owner == *owner) => entry,
        _ => return Err(SandboxError::not_found(format!("session {session_id} not found")).into()),
    };
    if entry.session.is_some() {
        if explicit_owner.is_some() {
            return Err(
                SandboxError::new(ErrorCode::InvalidState, "the session is not asleep").into(),
            );
        }
        return Ok(entry.state.clone());
    }
    let checkpoint = entry
        .checkpoint
        .clone()
        .ok_or_else(|| SandboxError::internal("a sleeping session has no checkpoint"))?;
    let limits = entry.limits.clone();
    let session: Arc<dyn Session> = restore_checkpoint_limited(checkpoint, &limits)
        .await?
        .into();
    entry.session = Some(session);
    entry.state.status = SessionStatus::Running;
    entry.state.updated_at = SystemTime::now();
    entry.touch();
    let woken = entry.state.clone();
    let owner = entry.owner.clone();
    drop(table);
    // The VFS keeps /etc/agent/env.json across sleep, so a wake does not rewrite it.
    state.usage.add(&owner, |c| c.sessions_woken += 1);
    persist(state).await?;
    Ok(woken)
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
        let session: Arc<dyn Session> =
            sandbox_session::restore_limited(record.root, &record.limits)
                .await?
                .into();
        let sid = session.id();
        let mut entry = entry(
            owner.clone(),
            session.clone(),
            TenantId::nil(),
            HashMap::new(),
            record.limits.clone(),
            record.allow_vm,
        );
        stamp(&state, &mut entry, &session).await?;
        write_forked_env_json(&session, sid, &record.limits).await?;
        admit_session(&state, &owner, entry).await?;
        state.usage.add(&owner, |c| c.sessions_created += 1);
        checkpoint_now(&state, &owner, sid, &session).await?;
        session_ids.push(sid);
    }
    persist(&state).await?;
    Ok(Json(ForkResponse { session_ids }))
}

/// The most checkpoint roots a session may keep pinned. Every pinned root keeps its
/// blobs alive through GC, so this bounds the storage one session can hold.
pub const MAX_CHECKPOINT_RETAIN: u32 = 256;

/// The logical bytes of checkpoints one key may keep pinned, across all its sessions.
pub const DEFAULT_CHECKPOINT_BYTES_PER_KEY: u64 = 1024 * 1024 * 1024;

/// The per-key checkpoint budget. `FRONTAL_SANDBOX_CHECKPOINT_BYTES_PER_KEY` sets it,
/// in bytes, and read once. An unset or invalid value gives the default.
fn checkpoint_budget_per_key() -> u64 {
    static BUDGET: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| {
        parse_checkpoint_budget(
            std::env::var("FRONTAL_SANDBOX_CHECKPOINT_BYTES_PER_KEY")
                .ok()
                .as_deref(),
        )
    })
}

/// Parses a byte budget. A zero or unparseable value is refused in favour of the default,
/// since a zero budget would refuse every checkpoint.
fn parse_checkpoint_budget(raw: Option<&str>) -> u64 {
    match raw.and_then(|value| value.trim().parse::<u64>().ok()) {
        Some(bytes) if bytes > 0 => bytes,
        _ => DEFAULT_CHECKPOINT_BYTES_PER_KEY,
    }
}

/// Drops the oldest pinned roots across `lists` until their total is within `budget`.
/// Each list keeps its newest root. When the newest roots alone are over budget,
/// nothing changes and this returns false.
fn evict_to_budget(lists: &mut [&mut Vec<CheckpointRoot>], budget: u64) -> bool {
    let newest_total: u64 = lists
        .iter()
        .filter_map(|roots| roots.last())
        .map(|root| root.bytes)
        .sum();
    if newest_total > budget {
        return false;
    }
    let mut total: u64 = lists
        .iter()
        .flat_map(|roots| roots.iter())
        .map(|root| root.bytes)
        .sum();
    while total > budget {
        // A total over budget with nothing left to drop does not fit, so the answer is no.
        let Some(list) = lists.iter_mut().find(|roots| roots.len() > 1) else {
            return false;
        };
        total -= list.remove(0).bytes;
    }
    true
}

/// Pins a checkpoint for the session's current state, and returns it. A state equal
/// to the newest checkpoint is not pinned again. The key's total pinned bytes stay
/// within the per-key budget, by dropping its oldest checkpoints. When
/// even the newest checkpoints do not fit, nothing is pinned and this fails with
/// `ResourceExhausted`.
async fn checkpoint_now(
    state: &AppState,
    owner: &KeyId,
    id: SessionId,
    session: &Arc<dyn Session>,
) -> Result<CheckpointRoot, ApiError> {
    let root = session.snapshot().await?;
    let bytes = session.fs().total_bytes().await.map_err(vfs_error)?;
    let pinned = CheckpointRoot { root, bytes };
    let mut table = state.sessions.0.write().await;
    let Some(entry) = table.get_mut(&id) else {
        return Ok(pinned);
    };
    let before = entry.roots.clone();
    pin_root(entry, pinned);
    let mut lists: Vec<&mut Vec<CheckpointRoot>> = table
        .values_mut()
        .filter(|entry| entry.owner == *owner)
        .map(|entry| &mut entry.roots)
        .collect();
    let budget = checkpoint_budget_per_key();
    if !evict_to_budget(&mut lists, budget) {
        drop(lists);
        if let Some(entry) = table.get_mut(&id) {
            entry.roots = before;
        }
        return Err(SandboxError::resource_exhausted(format!(
            "checkpoints would exceed the {budget}-byte limit for this key"
        ))
        .into());
    }
    Ok(pinned)
}

/// Adds a root as the newest checkpoint, and unpins the oldest beyond the limit.
fn pin_root(entry: &mut Entry, root: CheckpointRoot) {
    if entry
        .roots
        .last()
        .is_some_and(|last| last.root == root.root)
    {
        return;
    }
    entry.roots.push(root);
    let excess = entry
        .roots
        .len()
        .saturating_sub(entry.checkpoint_retain as usize);
    entry.roots.drain(..excess);
}

/// Pins a checkpoint after a command, when the session checkpoints per exec. A
/// checkpoint refused by the key's byte budget is skipped: the command already ran,
/// and older history is what gives way.
async fn checkpoint_after_exec(
    state: &AppState,
    owner: &KeyId,
    id: SessionId,
    session: &Arc<dyn Session>,
) -> Result<(), ApiError> {
    let per_exec = {
        let table = state.sessions.0.read().await;
        table
            .get(&id)
            .is_some_and(|entry| entry.checkpoint_policy == CheckpointPolicy::PerExec)
    };
    if !per_exec {
        return Ok(());
    }
    match checkpoint_now(state, owner, id, session).await {
        Err(ApiError(error)) if error.code == ErrorCode::ResourceExhausted => Ok(()),
        other => other.map(drop),
    }
}

async fn checkpoint_usage_of(state: &AppState, id: SessionId) -> CheckpointUsage {
    let table = state.sessions.0.read().await;
    match table.get(&id) {
        Some(entry) => CheckpointUsage {
            policy: entry.checkpoint_policy,
            count: entry.roots.len() as u32,
            retain: entry.checkpoint_retain,
            bytes: entry.roots.iter().map(|root| root.bytes).sum(),
        },
        None => CheckpointUsage {
            policy: CheckpointPolicy::default(),
            count: 0,
            retain: DEFAULT_CHECKPOINT_RETAIN,
            bytes: 0,
        },
    }
}

/// The root now current, and the checkpoints the session holds.
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckpointResponse {
    #[schema(value_type = String, pattern = "^[0-9a-f]{64}$")]
    pub root: ContentHash,
    pub checkpoints: CheckpointUsage,
}

/// Pins the session's current state as a checkpoint now. Manual checkpoints are
/// taken this way. A per-exec session also pins after each command.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/checkpoint",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "Checkpoint pinned", body = CheckpointResponse),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
        (status = 429, description = "The key's checkpoint byte budget is full", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn checkpoint_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<CheckpointResponse>, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    Ok(Json(checkpoint_locked(&state, &own).await?))
}

/// Pins the current state. The caller holds the session's gate.
async fn checkpoint_locked(state: &AppState, own: &Owned) -> Result<CheckpointResponse, ApiError> {
    let pinned = checkpoint_now(state, &own.owner, own.state.id, &own.session).await?;
    save_session(state, own.state.id, &own.session).await?;
    Ok(CheckpointResponse {
        root: pinned.root,
        checkpoints: checkpoint_usage_of(state, own.state.id).await,
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UndoRequest {
    /// How many checkpoints back to restore. Each step drops the newest one.
    #[serde(default = "default_undo_steps")]
    pub steps: u32,
}

fn default_undo_steps() -> u32 {
    1
}

/// Restores the filesystem from `steps` checkpoints ago (IMAGE.md §4.5). The current
/// state is pinned first, so undo can return to it. Processes are not kept: a Tier 1
/// session has none between commands.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/undo",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = UndoRequest,
    responses(
        (status = 200, description = "Filesystem restored", body = CheckpointResponse),
        (status = 400, description = "Not enough checkpoints to undo that far", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn undo_session(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Json(request): Json<UndoRequest>,
) -> Result<Json<CheckpointResponse>, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    Ok(Json(undo_locked(&state, &own, request.steps).await?))
}

/// Restores `steps` checkpoints back. The caller holds the session's gate.
async fn undo_locked(
    state: &AppState,
    own: &Owned,
    steps: u32,
) -> Result<CheckpointResponse, ApiError> {
    if steps == 0 {
        return Err(SandboxError::invalid_argument("steps must be at least 1").into());
    }
    checkpoint_now(state, &own.owner, own.state.id, &own.session).await?;
    let steps = steps as usize;
    let target = {
        let table = state.sessions.0.read().await;
        let entry = table
            .get(&own.state.id)
            .ok_or_else(|| SandboxError::not_found("session not found"))?;
        let len = entry.roots.len();
        if steps >= len {
            return Err(SandboxError::invalid_argument(format!(
                "only {} checkpoint(s) to undo to",
                len.saturating_sub(1)
            ))
            .into());
        }
        entry.roots[len - 1 - steps]
    };
    let current = own.session.checkpoint().await?;
    let restored: Arc<dyn Session> = restore_checkpoint_limited(
        Checkpoint {
            root: target.root,
            cwd: current.cwd,
            env: current.env,
        },
        &own.limits,
    )
    .await?
    .into();
    {
        let mut table = state.sessions.0.write().await;
        if let Some(entry) = table.get_mut(&own.state.id) {
            // The target becomes the newest checkpoint, so the steps after it go.
            let keep = entry.roots.len() - steps;
            entry.roots.truncate(keep);
            entry.session = Some(restored.clone());
            entry.state.updated_at = SystemTime::now();
        }
    }
    save_session(state, own.state.id, &restored).await?;
    Ok(CheckpointResponse {
        root: target.root,
        checkpoints: checkpoint_usage_of(state, own.state.id).await,
    })
}

/// The session's `/etc/agent/env.json`, as `sandbox info --json` prints it.
#[utoipa::path(
    get,
    path = "/v1/sessions/{id}/info",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    responses(
        (status = 200, description = "The environment, as env.json (schema 1)", body = serde_json::Value),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn session_info(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    let bytes = read_session_file(own.session.as_ref(), sandbox_session::ENV_JSON_PATH)
        .await?
        .ok_or_else(|| SandboxError::not_found("the session has no env.json"))?;
    let env = serde_json::from_slice(&bytes)
        .map_err(|e| SandboxError::internal(format!("env.json is damaged: {e}")))?;
    Ok(Json(env))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ForkCommandRequest {
    /// Children to create, from 1 to 64.
    pub count: u32,
    /// The command each child runs, as argv.
    pub argv: Vec<String>,
}

/// What one child did: the command's outcome, and how many files it changed.
#[derive(Debug, Serialize, ToSchema)]
pub struct ForkedChild {
    pub session_id: String,
    /// The exit code, or nothing when the command could not run.
    pub exit_code: Option<i32>,
    /// Why the command could not run, when it did not.
    pub error: Option<String>,
    pub changed_files: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ForkCommandResponse {
    pub children: Vec<ForkedChild>,
}

/// Forks the session's current state into `count` children, and runs `argv` in each
/// (IMAGE.md §4.5, `sandbox fork N -- <cmd>`). Each child is an independent session,
/// and its command is recorded in the child's own trace.
#[utoipa::path(
    post,
    path = "/v1/sessions/{id}/fork",
    tag = "sessions",
    params(("id" = String, Path, description = "Session ID (ULID)", pattern = "^[0-9A-HJKMNP-TV-Z]{26}$")),
    request_body = ForkCommandRequest,
    responses(
        (status = 200, description = "Children created and their commands run", body = ForkCommandResponse),
        (status = 400, description = "Invalid count or empty argv", body = ErrorBody),
        (status = 401, description = "Missing or invalid API key", body = ErrorBody),
        (status = 404, description = "No such session for this key", body = ErrorBody),
    ),
    security(("bearer" = []))
)]
pub async fn fork_command(
    State(state): State<AppState>,
    Extension(owner): Extension<KeyId>,
    Path(id): Path<String>,
    Json(request): Json<ForkCommandRequest>,
) -> Result<Json<ForkCommandResponse>, ApiError> {
    let own = owned(&state, &owner, &id).await?;
    let _gate = own.gate.lock().await;
    Ok(Json(
        fork_locked(&state, &own, request.count, &request.argv).await?,
    ))
}

/// Forks the current state into `count` children and runs `argv` in each. The caller
/// holds the session's gate.
async fn fork_locked(
    state: &AppState,
    own: &Owned,
    count: u32,
    argv: &[String],
) -> Result<ForkCommandResponse, ApiError> {
    if count == 0 || count > MAX_FORK_COUNT {
        return Err(SandboxError::invalid_argument(format!(
            "count must be between 1 and {MAX_FORK_COUNT}"
        ))
        .into());
    }
    if argv.is_empty() {
        return Err(SandboxError::invalid_argument("argv must not be empty").into());
    }
    let owner = &own.owner;
    // The children start from the current state, which is pinned first.
    let newest = checkpoint_now(state, owner, own.state.id, &own.session).await?;
    let mut children = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let child: Arc<dyn Session> = sandbox_session::restore_limited(newest.root, &own.limits)
            .await?
            .into();
        let child_id = child.id();
        let mut entry = entry(
            owner.clone(),
            child.clone(),
            TenantId::nil(),
            HashMap::new(),
            own.limits.clone(),
            own.state.allow_vm,
        );
        stamp(state, &mut entry, &child).await?;
        write_forked_env_json(&child, child_id, &own.limits).await?;
        admit_session(state, owner, entry).await?;
        state.usage.add(owner, |c| c.sessions_created += 1);
        checkpoint_now(state, owner, child_id, &child).await?;

        let exec = ExecRequest {
            argv: argv.to_vec(),
            ..Default::default()
        };
        let before = committed_root(child.as_ref()).await;
        let random_state = Some(child.random_state().await);
        let result = child.exec(exec.clone()).await;
        state.usage.add(owner, |c| {
            c.execs += 1;
            if let Ok(out) = &result {
                c.exec_wall_ms += out.wall_ms;
            }
        });
        let files = files_since(child.as_ref(), before).await;
        let changed_files = files.as_ref().map_or(0, Vec::len);
        record_exec(state, child_id, &exec, &result, files, random_state).await;
        checkpoint_after_exec(state, owner, child_id, &child).await?;
        save_session(state, child_id, &child).await?;
        children.push(ForkedChild {
            session_id: child_id.to_string(),
            exit_code: result.as_ref().ok().map(|out| out.exit_code),
            error: result.as_ref().err().map(|e| e.message.clone()),
            changed_files,
        });
    }
    persist(state).await?;
    Ok(ForkCommandResponse { children })
}

/// Runs `sandbox checkpoint`, `sandbox undo [N]`, or `sandbox fork N -- <cmd>` typed
/// as an exec from inside the session. Returns `None` for any other argv, which then
/// runs in the shell as usual. The caller holds the session's gate.
async fn host_sandbox_command(
    state: &AppState,
    own: &Owned,
    argv: &[String],
) -> Option<ExecResult> {
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let outcome = match args.as_slice() {
        ["sandbox", "checkpoint"] => checkpoint_locked(state, own)
            .await
            .map(|r| r.root.to_string())
            .map_err(|ApiError(e)| e),
        ["sandbox", "undo"] => undo_locked(state, own, 1)
            .await
            .map(|r| r.root.to_string())
            .map_err(|ApiError(e)| e),
        ["sandbox", "undo", steps] => {
            let Ok(steps) = steps.parse::<u32>() else {
                return Some(usage_error("sandbox undo: steps must be a number"));
            };
            undo_locked(state, own, steps)
                .await
                .map(|r| r.root.to_string())
                .map_err(|ApiError(e)| e)
        }
        ["sandbox", "fork", count, "--", command @ ..] if !command.is_empty() => {
            let Ok(count) = count.parse::<u32>() else {
                return Some(usage_error("sandbox fork: count must be a number"));
            };
            let argv: Vec<String> = command.iter().map(|s| s.to_string()).collect();
            return Some(match fork_locked(state, own, count, &argv).await {
                Ok(response) => {
                    let failed = response.children.iter().any(|c| c.exit_code != Some(0));
                    let lines: String = response
                        .children
                        .iter()
                        .map(|child| {
                            let outcome = match (child.exit_code, &child.error) {
                                (Some(0), _) => "ok".to_string(),
                                (Some(code), _) => format!("exit {code}"),
                                (None, error) => {
                                    format!("error: {}", error.as_deref().unwrap_or("unknown"))
                                }
                            };
                            format!(
                                "{} {outcome} changed={}\n",
                                child.session_id, child.changed_files
                            )
                        })
                        .collect();
                    ExecResult {
                        exit_code: i32::from(failed),
                        stdout: lines.into_bytes(),
                        stderr: Vec::new(),
                        cpu_ms: 0,
                        wall_ms: 0,
                    }
                }
                Err(ApiError(error)) => failed_command(error),
            });
        }
        ["sandbox", "checkpoint" | "undo" | "fork", ..] => {
            return Some(usage_error(
                "usage: sandbox checkpoint | sandbox undo [N] | sandbox fork N -- <cmd>",
            ))
        }
        _ => return None,
    };
    Some(match outcome {
        Ok(line) => ExecResult {
            exit_code: 0,
            stdout: format!("{line}\n").into_bytes(),
            stderr: Vec::new(),
            cpu_ms: 0,
            wall_ms: 0,
        },
        Err(error) => failed_command(error),
    })
}

/// A command that failed as a host operation: exit 1, with the reason on stderr.
fn failed_command(error: SandboxError) -> ExecResult {
    ExecResult {
        exit_code: 1,
        stdout: Vec::new(),
        stderr: format!("{}\n", error.message).into_bytes(),
        cpu_ms: 0,
        wall_ms: 0,
    }
}

fn usage_error(message: &str) -> ExecResult {
    ExecResult {
        exit_code: 2,
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
        cpu_ms: 0,
        wall_ms: 0,
    }
}

#[cfg(test)]
mod checkpoint_budget_tests {
    use super::*;
    use sandbox_types::hash::ContentHash;

    #[test]
    fn the_spill_budget_setting_falls_back_to_the_default_when_unusable() {
        assert_eq!(parse_spill_budget(None), DEFAULT_SPILLED_BYTES_PER_SESSION);
        assert_eq!(
            parse_spill_budget(Some("x")),
            DEFAULT_SPILLED_BYTES_PER_SESSION
        );
        assert_eq!(
            parse_spill_budget(Some("0")),
            DEFAULT_SPILLED_BYTES_PER_SESSION
        );
        assert_eq!(parse_spill_budget(Some(" 1024 ")), 1024);
    }

    #[test]
    fn the_budget_setting_falls_back_to_the_default_when_unusable() {
        assert_eq!(
            parse_checkpoint_budget(None),
            DEFAULT_CHECKPOINT_BYTES_PER_KEY
        );
        assert_eq!(
            parse_checkpoint_budget(Some("abc")),
            DEFAULT_CHECKPOINT_BYTES_PER_KEY
        );
        assert_eq!(
            parse_checkpoint_budget(Some("0")),
            DEFAULT_CHECKPOINT_BYTES_PER_KEY
        );
        assert_eq!(parse_checkpoint_budget(Some(" 4096 ")), 4096);
    }

    fn root(byte: u8, bytes: u64) -> CheckpointRoot {
        CheckpointRoot {
            root: ContentHash::new(&[byte]),
            bytes,
        }
    }

    #[test]
    fn eviction_drops_the_oldest_roots_until_the_total_fits() {
        let mut a = vec![root(1, 40), root(2, 40), root(3, 40)];
        let mut b = vec![root(4, 30), root(5, 30)];
        assert!(evict_to_budget(&mut [&mut a, &mut b], 100));
        assert!(
            a.iter().map(|r| r.bytes).sum::<u64>() + b.iter().map(|r| r.bytes).sum::<u64>() <= 100
        );
        // Every list keeps its newest root.
        assert_eq!(a.last().unwrap().root, ContentHash::new(&[3]));
        assert_eq!(b.last().unwrap().root, ContentHash::new(&[5]));
    }

    #[test]
    fn eviction_changes_nothing_when_the_newest_roots_do_not_fit() {
        let mut a = vec![root(1, 10), root(2, 60)];
        let mut b = vec![root(3, 60)];
        assert!(!evict_to_budget(&mut [&mut a, &mut b], 100));
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
    }
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
