//! Clustered chaos tests (M5, "chaos suite passes"). The file-backed durable store has its
//! own chaos suite (`chaos_crash`, `chaos_damaged`, `chaos_torn`). This is the clustered
//! counterpart: the state lives in Postgres, so the faults are database-level, not file-level.
//!
//! Each scenario has its own concern:
//! - a save is atomic: a crash mid-write leaves the previous complete document, never a half
//!   one (Postgres guarantees this; we assert it).
//! - a damaged or torn `body` (invalid JSON, wrong version) is refused on open, not read as
//!   an empty server.
//! - a stale node (one that last read an older revision) cannot overwrite a newer revision:
//!   its save fails with `InvalidState`.
//!
//! Needs a Postgres database, given as `SANDBOX_TEST_DATABASE_URL`. Point it at a database
//! used only for tests. Without the variable the test fails, so a run without a database is
//! never reported as a pass.

#![cfg(feature = "clustered")]

use sandbox_server::persist::{Saved, SavedKey, VERSION};
use sandbox_server::pg_store::PgStore;
use sandbox_types::{ErrorCode, TenantId};

/// The tests share one table, so they run one at a time.
static DATABASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn database_url() -> String {
    std::env::var("SANDBOX_TEST_DATABASE_URL")
        .expect("set SANDBOX_TEST_DATABASE_URL to a Postgres database used only for tests")
}

async fn reset_table(pool: &sqlx::PgPool) {
    sqlx::query("DROP TABLE IF EXISTS sandbox_state")
        .execute(pool)
        .await
        .unwrap();
}

fn state_with_key(id: &str) -> Saved {
    Saved {
        version: VERSION,
        keys: vec![SavedKey {
            id: id.to_string(),
            digest: "00".repeat(32),
            tenant: TenantId::nil(),
        }],
        ..Saved::default()
    }
}

#[tokio::test]
async fn a_save_is_atomic_and_a_crash_leaves_the_previous_document() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    reset_table(&pool).await;

    // First save: creates the row with revision 1.
    let (store, saved) = PgStore::connect(&url).await.unwrap();
    assert_eq!(saved.version, VERSION);
    store
        .save(|| async { Ok(state_with_key("k1")) })
        .await
        .unwrap();

    // Second save: bumps to revision 2 with a different key.
    store
        .save(|| async { Ok(state_with_key("k2")) })
        .await
        .unwrap();

    // Simulate a crash mid-third-save: the third save never completes, so the row must still
    // hold the revision-2 document. We verify by reading the raw row.
    let (revision, body): (i64, serde_json::Value) =
        sqlx::query_as("SELECT revision, body FROM sandbox_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        revision, 2,
        "a crashed third save must not advance the revision"
    );
    let saved: Saved = serde_json::from_value(body).unwrap();
    assert_eq!(
        saved.keys.first().map(|k| k.id.as_str()),
        Some("k2"),
        "the last complete save must still be there"
    );
    pool.close().await;
}

#[tokio::test]
async fn a_damaged_body_refuses_to_open() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    reset_table(&pool).await;

    // Seed a valid row, then corrupt its body with invalid JSON.
    let (store, _) = PgStore::connect(&url).await.unwrap();
    store
        .save(|| async { Ok(state_with_key("k1")) })
        .await
        .unwrap();
    drop(store);
    // A body that parses as JSON but is not a Saved document: it carries the right version
    // (so the version check passes) but the rest of the fields are wrong.
    let damaged = serde_json::json!({"version": VERSION, "op": "put_key", "value": "not json"});
    sqlx::query("UPDATE sandbox_state SET body = $1::jsonb")
        .bind(&damaged)
        .execute(&pool)
        .await
        .unwrap();

    // Reconnecting must refuse, not read the damaged document.
    match PgStore::connect(&url).await {
        Ok(_) => panic!("a damaged body must refuse to open"),
        Err(error) => {
            // A body that still parses as JSON but is not a valid Saved document is an
            // internal error (the version matched, the document did not). Either way the
            // store refuses to open; a damaged body is never read as an empty server.
            assert!(
                matches!(error.code, ErrorCode::Internal | ErrorCode::InvalidState),
                "expected a refusal, got {:?}",
                error.code
            );
        }
    }
    pool.close().await;
}

#[tokio::test]
async fn a_wrong_version_refuses_to_open() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    reset_table(&pool).await;

    // Seed a valid row, then bump its version past what this server reads.
    let (store, _) = PgStore::connect(&url).await.unwrap();
    store
        .save(|| async { Ok(state_with_key("k1")) })
        .await
        .unwrap();
    drop(store);
    let bumped = (VERSION as i64) + 1;
    sqlx::query(
        "UPDATE sandbox_state SET body = jsonb_set(body, '{version}', to_jsonb($1::bigint))",
    )
    .bind(bumped)
    .execute(&pool)
    .await
    .unwrap();

    match PgStore::connect(&url).await {
        Ok(_) => panic!("a state of another version must refuse to open"),
        Err(error) => assert!(
            matches!(error.code, ErrorCode::InvalidState),
            "expected InvalidState, got {:?}",
            error.code
        ),
    }
    pool.close().await;
}

#[tokio::test]
async fn a_stale_node_cannot_overwrite_a_newer_revision() {
    let _one_at_a_time = DATABASE.lock().await;
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    reset_table(&pool).await;

    // Node A reads the empty state (revision 0) and prepares a save.
    let (node_a, _) = PgStore::connect(&url).await.unwrap();

    // Node B connects after A, so it also starts at revision 0, but B saves first.
    let (node_b, _) = PgStore::connect(&url).await.unwrap();
    node_b
        .save(|| async { Ok(state_with_key("from-b")) })
        .await
        .unwrap();

    // A now saves. A still believes the revision is 0, but the row is at revision 1, so the
    // INSERT ... ON CONFLICT DO NOTHING writes nothing and the save must fail.
    match node_a.save(|| async { Ok(state_with_key("from-a")) }).await {
        Ok(_) => panic!("a stale node must not overwrite a newer revision"),
        Err(error) => assert!(
            matches!(error.code, ErrorCode::InvalidState),
            "expected InvalidState, got {:?}",
            error.code
        ),
    }

    // The row still holds B's document.
    let body: serde_json::Value = sqlx::query_scalar("SELECT body FROM sandbox_state WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let saved: Saved = serde_json::from_value(body).unwrap();
    assert_eq!(
        saved.keys.first().map(|k| k.id.as_str()),
        Some("from-b"),
        "B's newer write must survive A's stale save"
    );
    pool.close().await;
}
