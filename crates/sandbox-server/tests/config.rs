//! Unit tests moved out of src/config.rs.

use sandbox_server::config::{load, ConfigError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn clear_env() {
    for k in [
        "SANDBOX_SERVER_ADDR",
        "SANDBOX_SERVER_DATA_DIR",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "SANDBOX_SERVER_TELEMETRY_OTLP_ENDPOINT",
        "SANDBOX_DATABASE_URL",
        "SANDBOX_SCHEDULER_METADATA_DSN",
        "SANDBOX_IDLE_SLEEP_SECS",
        "SANDBOX_SCHEDULER_IDLE_SLEEP_AFTER_SECONDS",
        "SANDBOX_OBJECT_STORE_BUCKET",
        "SANDBOX_OBJECT_STORE_ENDPOINT",
        "HOME",
    ] {
        std::env::remove_var(k);
    }
}

#[test]
fn parses_infra_dev_example() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("HOME", "/tmp/sandbox-test-home");
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infrastructure/config/examples/dev.toml");
    let resolved = load(Some(&path)).expect("dev config");
    clear_env();
    assert_eq!(resolved.addr, "127.0.0.1:8080");
    assert!(resolved
        .data_dir
        .expect("data_dir")
        .ends_with(".local/state/sandbox/dev"));
    assert_eq!(resolved.idle_sleep_secs, 300);
    assert_eq!(resolved.otel_endpoint, None);
}

#[test]
fn parses_infra_production_example() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../infrastructure/config/examples/production.toml");
    let resolved = load(Some(&path)).expect("prod config");
    clear_env();
    assert_eq!(resolved.addr, "0.0.0.0:8080");
    assert_eq!(
        resolved.data_dir.expect("data_dir"),
        PathBuf::from("/var/lib/sandbox")
    );
    assert_eq!(resolved.idle_sleep_secs, 300);
    assert_eq!(
        resolved.database_url.as_deref(),
        Some("postgres://sandbox:<PASSWORD>@db.internal:5432/sandbox?sslmode=require")
    );
    assert_eq!(
        resolved.otel_endpoint.as_deref(),
        Some("http://otel-collector.production:4317")
    );
    assert_eq!(
        resolved.object_store_bucket.as_deref(),
        Some("sandbox-cas-production")
    );
}

#[test]
fn env_overrides_file() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("HOME", "/tmp/home");
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infrastructure/config/examples/dev.toml");
    std::env::set_var("SANDBOX_SERVER_ADDR", "127.0.0.1:9999");
    std::env::set_var("SANDBOX_IDLE_SLEEP_SECS", "42");
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel:4318");
    let resolved = load(Some(&path)).expect("config");
    clear_env();
    assert_eq!(resolved.addr, "127.0.0.1:9999");
    assert_eq!(resolved.idle_sleep_secs, 42);
    assert_eq!(resolved.otel_endpoint.as_deref(), Some("http://otel:4318"));
}

#[test]
fn explicit_missing_file_is_error() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    let err = load(Some(Path::new("/no/such/sandbox-config.toml"))).unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }));
}

#[test]
fn missing_default_file_uses_defaults() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    let resolved = load(None).expect("defaults");
    clear_env();
    assert_eq!(resolved.addr, "0.0.0.0:8080");
    assert_eq!(resolved.gc_interval_secs, 600);
    assert_eq!(resolved.idle_sleep_secs, 300);
}
