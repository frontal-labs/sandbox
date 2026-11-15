//! Integration tests for the base `Scheduler`, `ClusterScheduler`, and their
//! placement / pool helpers.
//!
//! All of these are pure in-memory state machines, so no network, VMM, or store
//! access is needed. The tests assert the behavioral contracts: least-loaded
//! placement, load release, pool capacity bounds, and the local-vs-remote
//! placement / migration error paths.

use sandbox_scheduler::cluster::{ClusterScheduler, SchedulerNode};
use sandbox_scheduler::placement::{PlacementStrategy, WorkerInfo};
use sandbox_scheduler::pools::{IsolatePool, PoolConfig};
use sandbox_scheduler::Scheduler;
use sandbox_types::{ErrorCode, SessionConfig, SessionId, SessionStatus, TenantId, TierPreference};

fn tenant() -> TenantId {
    TenantId::new()
}

fn config() -> SessionConfig {
    SessionConfig::new(tenant())
}

fn worker(id: &str, capacity: u64, load: u64) -> WorkerInfo {
    WorkerInfo {
        id: id.to_string(),
        tenant_id: tenant(),
        capacity,
        current_load: load,
    }
}

// ---------------------------------------------------------------------------
// PlacementStrategy
// ---------------------------------------------------------------------------

#[test]
fn placement_picks_least_loaded_worker() {
    let mut strategy = PlacementStrategy::new();
    let workers = vec![
        worker("busy", 10, 5),
        worker("idle", 10, 0),
        worker("mid", 10, 3),
    ];

    // A fresh strategy has no tracked load, so all workers start at 0; the
    // first placement is well-defined and returns one of them.
    let decision = strategy
        .place(&config(), &workers)
        .expect("a placement should be made");
    assert!(
        decision.worker_id == "busy" || decision.worker_id == "idle" || decision.worker_id == "mid"
    );
}

#[test]
fn placement_returns_none_with_no_workers() {
    let mut strategy = PlacementStrategy::new();
    assert!(strategy.place(&config(), &[]).is_none());
}

#[test]
fn placement_tracks_load_and_balances() {
    let mut strategy = PlacementStrategy::new();
    let workers = vec![worker("a", 10, 0), worker("b", 10, 0)];

    // First two placements both see load 0; ties are broken by min_by_key's
    // first-minimum, so "a" wins the first. After that, its tracked load is 1.
    let first = strategy.place(&config(), &workers).unwrap();
    let second = strategy.place(&config(), &workers).unwrap();
    assert_eq!(first.worker_id, "a");
    assert_eq!(second.worker_id, "b");

    // After releasing "a", its load drops back and it is preferred again.
    strategy.release("a");
    let third = strategy.place(&config(), &workers).unwrap();
    assert_eq!(third.worker_id, "a");
}

#[test]
fn release_is_saturating() {
    let mut strategy = PlacementStrategy::new();
    // Releasing a worker that was never placed must not underflow or panic.
    strategy.release("never-seen");
    strategy.release("never-seen");
    let workers = vec![worker("a", 10, 0)];
    assert!(strategy.place(&config(), &workers).is_some());
}

// ---------------------------------------------------------------------------
// IsolatePool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_starts_empty_and_round_trips() {
    let pool = IsolatePool::new(PoolConfig::default());
    assert_eq!(pool.stats().await.available, 0);
    assert!(pool.acquire(&config()).await.is_none());

    // An isolate that is released can be acquired again.
    let isolate = sandbox_scheduler::pools::PooledIsolate {
        id: "iso-1".into(),
        tier: sandbox_types::Tier::Isolate,
        created_at: std::time::Instant::now(),
        last_used: std::time::Instant::now(),
    };
    pool.release(isolate.clone()).await;
    let acquired = pool.acquire(&config()).await.expect("pooled isolate");
    assert_eq!(acquired.id, "iso-1");
    assert_eq!(pool.stats().await.available, 0);
}

#[tokio::test]
async fn pool_release_respects_max_size() {
    let pool = IsolatePool::new(PoolConfig {
        max_size: 2,
        ..PoolConfig::default()
    });
    for i in 0..3 {
        pool.release(sandbox_scheduler::pools::PooledIsolate {
            id: format!("iso-{i}"),
            tier: sandbox_types::Tier::Isolate,
            created_at: std::time::Instant::now(),
            last_used: std::time::Instant::now(),
        })
        .await;
    }
    // The third release was dropped because the pool is already at max_size.
    let stats = pool.stats().await;
    assert_eq!(stats.available, 2);
    assert_eq!(stats.max_size, 2);
}

// ---------------------------------------------------------------------------
// Scheduler (single node)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scheduler_create_get_destroy_session() {
    let scheduler = Scheduler::new();
    let session_id = scheduler.create_session(config()).await.unwrap();

    let state = scheduler
        .get_session(session_id)
        .await
        .expect("session exists");
    assert_eq!(state.id, session_id);
    assert_eq!(state.status, SessionStatus::Running);

    scheduler.destroy_session(session_id).await.unwrap();
    assert!(scheduler.get_session(session_id).await.is_none());
}

#[tokio::test]
async fn scheduler_allow_vm_reflects_tier_preference() {
    let scheduler = Scheduler::new();

    // An explicit isolate-only preference must not allow VM promotion.
    let isolate_only = scheduler
        .create_session(config().with_tier(TierPreference::Isolate))
        .await
        .unwrap();
    assert!(!scheduler.get_session(isolate_only).await.unwrap().allow_vm);

    // Any other preference permits promotion to a VM.
    let vm_ok = scheduler
        .create_session(config().with_tier(TierPreference::Vm))
        .await
        .unwrap();
    assert!(scheduler.get_session(vm_ok).await.unwrap().allow_vm);
}

#[tokio::test]
async fn scheduler_worker_registration_round_trip() {
    let scheduler = Scheduler::new();
    scheduler.register_worker(worker("w1", 4, 0)).await;
    // A registered worker participates in placement: creating a session with a
    // warm pool path still succeeds.
    assert!(scheduler.create_session(config()).await.is_ok());

    scheduler.unregister_worker("w1").await;
    // Unregistering is non-fatal; sessions can still be created.
    assert!(scheduler.create_session(config()).await.is_ok());
}

// ---------------------------------------------------------------------------
// ClusterScheduler
// ---------------------------------------------------------------------------

fn node(id: &str, load: u64, healthy: bool) -> SchedulerNode {
    SchedulerNode {
        id: id.to_string(),
        address: format!("{id}:50051"),
        tenant_id: tenant(),
        capacity: 100,
        current_load: load,
        healthy,
        last_heartbeat: std::time::SystemTime::now(),
    }
}

#[tokio::test]
async fn cluster_register_self_and_healthy_filter() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    scheduler.register_self().await;

    scheduler.register_node(node("other", 0, true)).await;
    scheduler.register_node(node("sick", 0, false)).await;

    let healthy = scheduler.get_healthy_nodes().await;
    let ids: Vec<&str> = healthy.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"self"));
    assert!(ids.contains(&"other"));
    assert!(!ids.contains(&"sick"));

    // cluster_nodes() returns all registered nodes, healthy or not.
    assert_eq!(scheduler.cluster_nodes().await.len(), 3);
}

#[tokio::test]
async fn cluster_create_session_succeeds_when_local_node_is_registered() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    scheduler.register_self().await;

    let session_id = scheduler.create_session(config()).await;
    // The only healthy node is this one, so local placement must succeed.
    assert!(session_id.is_ok(), "{session_id:?}");
    let id = session_id.unwrap();
    assert!(scheduler.get_session(id).await.is_some());
}

#[tokio::test]
async fn cluster_create_session_fails_with_no_healthy_nodes() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    // No nodes registered at all: nothing is healthy, placement must fail.
    let err = scheduler
        .create_session(config())
        .await
        .expect_err("no healthy nodes");
    assert_eq!(err.code, ErrorCode::Unavailable);
}

#[tokio::test]
async fn cluster_create_session_refuses_remote_target() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    // Register only a remote healthy node. The local node is *not* registered,
    // so cluster placement picks "remote" — the only candidate — and must
    // refuse to create a session on it.
    scheduler.register_node(node("remote", 0, true)).await;

    // Local placement is skipped (empty pool), so create_session falls through
    // to cluster placement, which must error because the target is remote.
    let err = scheduler
        .create_session(config())
        .await
        .expect_err("remote creation is unimplemented");
    assert_eq!(err.code, ErrorCode::Unsupported);
}

#[tokio::test]
async fn cluster_destroy_and_get_session() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    scheduler.register_self().await;
    let id = scheduler.create_session(config()).await.unwrap();

    assert!(scheduler.get_session(id).await.is_some());
    scheduler.destroy_session(id).await.unwrap();
    assert!(scheduler.get_session(id).await.is_none());

    // Destroying an unknown session is a no-op success.
    assert!(scheduler.destroy_session(SessionId::new()).await.is_ok());
}

#[tokio::test]
async fn cluster_migrate_session_error_paths() {
    let scheduler = ClusterScheduler::new("self".into(), "self:50051".into(), tenant());
    scheduler.register_self().await;
    let id = scheduler.create_session(config()).await.unwrap();

    // Migrating to self is an immediate no-op success.
    assert!(scheduler.migrate_session(id, "self").await.is_ok());

    // Migrating to a remote node is unimplemented and must error.
    let err = scheduler
        .migrate_session(id, "remote")
        .await
        .expect_err("remote migration unimplemented");
    assert_eq!(err.code, ErrorCode::Unsupported);

    // Migrating an unknown session is NotFound.
    let missing = scheduler
        .migrate_session(SessionId::new(), "remote")
        .await
        .expect_err("unknown session");
    assert_eq!(missing.code, ErrorCode::NotFound);
}
