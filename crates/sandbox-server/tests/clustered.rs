//! The Postgres state store (PLAN.md §4.8, M5). Needs a Postgres database, given
//! as `SANDBOX_TEST_DATABASE_URL`. The test resets the `sandbox_state` table, so
//! point it at a database used only for tests. Without the variable the test fails,
//! so a run without a database is never reported as a pass.
#![cfg(feature = "clustered")]

use sandbox_server::persist::{Saved, SavedKey, VERSION};
use sandbox_server::pg_store::PgStore;
use sandbox_types::ErrorCode;

/// The tests share one table, so they run one at a time.
static DATABASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn database_url() -> String {
    std::env::var("SANDBOX_TEST_DATABASE_URL")
        .expect("set SANDBOX_TEST_DATABASE_URL to a Postgres database used only for tests")
}

fn state_with_key(id: &str) -> Saved {
    Saved {
        version: VERSION,
        keys: vec![SavedKey {
            id: id.to_string(),
            digest: "00".repeat(32),
        }],
        ..Saved::default()
    }
}

#[tokio::test]
async fn saved_state_survives_a_restart_and_a_stale_node_cannot_overwrite_it() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    // A clean table, so the test does not see another run's state.
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS sandbox_state")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    // An empty database has an empty state.
    let (node_a, saved) = PgStore::connect(&url).await.unwrap();
    assert!(saved.keys.is_empty());
    node_a
        .save(|| async { state_with_key("first") })
        .await
        .unwrap();

    // A restart (a new connection) reads what was saved.
    let (restarted, saved) = PgStore::connect(&url).await.unwrap();
    assert_eq!(saved.keys.len(), 1);
    assert_eq!(saved.keys[0].id, "first");

    // Another node reads the same state, then writes. Node A still holds the
    // earlier revision, so its save is refused and writes nothing.
    let (node_b, _) = PgStore::connect(&url).await.unwrap();
    node_b
        .save(|| async { state_with_key("second") })
        .await
        .unwrap();
    let err = restarted
        .save(|| async { state_with_key("stale") })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState, "{}", err.message);

    // The refused write left node B's state in place.
    let (_, saved) = PgStore::connect(&url).await.unwrap();
    assert_eq!(saved.keys[0].id, "second");

    // Once the stale node reloads, it can save again.
    let (reloaded, _) = PgStore::connect(&url).await.unwrap();
    reloaded
        .save(|| async { state_with_key("third") })
        .await
        .unwrap();
    let (_, saved) = PgStore::connect(&url).await.unwrap();
    assert_eq!(saved.keys[0].id, "third");
}

#[tokio::test]
async fn a_state_from_another_version_is_refused_not_misread() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS sandbox_state")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE sandbox_state (id SMALLINT PRIMARY KEY, revision BIGINT NOT NULL, body JSONB NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO sandbox_state VALUES (1, 1, '{\"version\": 999}')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let err = match PgStore::connect(&url).await {
        Ok(_) => panic!("a state of another version must be refused"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::InvalidState, "{}", err.message);
}
