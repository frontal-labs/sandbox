//! Unit tests moved out of src/config.rs.

use sandbox_server::config::{
    ignored_security_settings, load, ConfigError, ResolvedServeConfig, ServerConfigFile,
};
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
        "SANDBOX_RATE_LIMIT_RPS",
        "SANDBOX_TLS_CERT",
        "SANDBOX_TLS_KEY",
        "SANDBOX_TLS_CA",
        "SANDBOX_TLS_REQUIRE_CLIENT_AUTH",
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
fn debug_output_hides_the_database_password() {
    let config = ResolvedServeConfig {
        database_url: Some("postgres://sandbox:hunter2@db.internal/sandbox".to_string()),
        ..ResolvedServeConfig::default()
    };
    let shown = format!("{config:?}");
    assert!(!shown.contains("hunter2"), "password leaked: {shown}");
    assert!(shown.contains("<redacted>"));
}

#[test]
fn unimplemented_auth_settings_are_reported() {
    let file: ServerConfigFile =
        toml::from_str("[server.auth]\nrate_limit_rps = 100\n").expect("parses");
    let warnings = ignored_security_settings(&file);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("[server.auth]"));
}

#[test]
fn tls_is_not_reported_when_configured() {
    // TLS is implemented now; enabling it must not be flagged as ignored.
    let file: ServerConfigFile =
        toml::from_str("[server.tls]\nenabled = true\ncert = \"/c\"\nkey = \"/k\"\n")
            .expect("parses");
    assert!(ignored_security_settings(&file).is_empty());
    let mut resolved = ResolvedServeConfig::default();
    sandbox_server::config::apply_file(&mut resolved, &file);
    assert!(resolved.tls.enabled);
    assert_eq!(
        resolved.tls.cert.as_deref(),
        Some(std::path::Path::new("/c"))
    );
}

#[test]
fn disabled_tls_reports_nothing() {
    let file: ServerConfigFile = toml::from_str("[server.tls]\nenabled = false\n").expect("parses");
    assert!(ignored_security_settings(&file).is_empty());
    assert!(ignored_security_settings(&ServerConfigFile::default()).is_empty());
}

#[test]
fn tls_defaults_to_disabled() {
    let mut resolved = ResolvedServeConfig::default();
    sandbox_server::config::apply_file(&mut resolved, &ServerConfigFile::default());
    assert!(!resolved.tls.enabled);
    assert!(resolved.tls.cert.is_none());
    assert!(!resolved.tls.require_client_auth);
}

#[test]
fn tls_unknown_field_is_rejected() {
    // A typo in `cert` must be a config error, not silently plaintext.
    let err = toml::from_str::<ServerConfigFile>(
        "[server.tls]\nenabled = true\ncert = \"/c\"\nkey = \"/k\"\ncertfile = \"/x\"\n",
    )
    .expect_err("unknown tls field must fail");
    assert!(
        err.to_string().contains("certfile"),
        "error names the typo: {err}"
    );
}

#[test]
fn tls_client_auth_parses_ca() {
    let file: ServerConfigFile = toml::from_str(
        "[server.tls]\nenabled = true\ncert = \"/c\"\nkey = \"/k\"\nca = \"/ca\"\nrequire_client_auth = true\n",
    )
    .expect("parses");
    let mut resolved = ResolvedServeConfig::default();
    sandbox_server::config::apply_file(&mut resolved, &file);
    assert!(resolved.tls.require_client_auth);
    assert_eq!(
        resolved.tls.ca.as_deref(),
        Some(std::path::Path::new("/ca"))
    );
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

#[test]
fn parses_rate_limit_from_toml() {
    let file: ServerConfigFile =
        toml::from_str("[server]\nrate_limit = { requests_per_second = 500 }\n").expect("parses");
    assert_eq!(
        file.server.rate_limit.and_then(|rl| rl.requests_per_second),
        Some(500)
    );
}

#[test]
fn rate_limit_zero_in_toml() {
    let file: ServerConfigFile =
        toml::from_str("[server]\nrate_limit = { requests_per_second = 0 }\n").expect("parses");
    assert_eq!(
        file.server.rate_limit.and_then(|rl| rl.requests_per_second),
        Some(0)
    );
}

#[test]
fn rate_limit_absent_by_default() {
    let file: ServerConfigFile = toml::from_str("[server]\n").expect("parses");
    assert!(file.server.rate_limit.is_none());
}

#[test]
fn rate_limit_rejects_unknown_field() {
    let result: Result<ServerConfigFile, _> =
        toml::from_str("[server]\nrate_limit = { burst = 10 }\n");
    assert!(result.is_err(), "unknown field should be rejected");
}

#[test]
fn rate_limit_env_overrides_file() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("HOME", "/tmp/home");
    std::env::set_var("SANDBOX_RATE_LIMIT_RPS", "999");
    let resolved = load(None).expect("defaults");
    clear_env();
    assert_eq!(resolved.rate_limit_rps, Some(999));
}

#[test]
fn rate_limit_not_in_ignored_security_settings() {
    // Rate limiting is now implemented, so it should not appear in the
    // "ignored" list even when configured.
    let file: ServerConfigFile =
        toml::from_str("[server]\nrate_limit = { requests_per_second = 100 }\n").expect("parses");
    let warnings = ignored_security_settings(&file);
    assert!(
        !warnings.iter().any(|w| w.contains("rate_limit")),
        "rate_limit should not be listed as ignored: {warnings:?}"
    );
}

#[test]
fn tls_require_client_auth_env_true() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("SANDBOX_TLS_REQUIRE_CLIENT_AUTH", "true");
    let resolved = load(None).expect("defaults");
    clear_env();
    assert!(resolved.tls.require_client_auth);
}

#[test]
fn tls_require_client_auth_env_false() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("SANDBOX_TLS_REQUIRE_CLIENT_AUTH", "false");
    let resolved = load(None).expect("defaults");
    clear_env();
    assert!(!resolved.tls.require_client_auth);
}

#[test]
fn tls_require_client_auth_env_invalid_value_is_error() {
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("SANDBOX_TLS_REQUIRE_CLIENT_AUTH", "yes");
    let err = load(None).unwrap_err();
    clear_env();
    let msg = err.to_string();
    assert!(
        msg.contains("SANDBOX_TLS_REQUIRE_CLIENT_AUTH"),
        "error names the variable: {msg}"
    );
    assert!(msg.contains("yes"), "error names the bad value: {msg}");
}

#[test]
fn tls_require_client_auth_env_invalid_zero_is_error() {
    // "0" used to silently mean false; now it's an explicit error so a
    // typo cannot disable client authentication.
    let _g = env_lock().lock().expect("lock");
    clear_env();
    std::env::set_var("SANDBOX_TLS_REQUIRE_CLIENT_AUTH", "0");
    let err = load(None).unwrap_err();
    clear_env();
    assert!(matches!(err, ConfigError::InvalidBool { .. }));
}
