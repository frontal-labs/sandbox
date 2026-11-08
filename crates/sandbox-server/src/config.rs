//! Server configuration loaded from TOML (infrastructure/PLAN.md §4).
//!
//! Precedence for resolved serve fields (highest wins): explicit CLI flags,
//! `SANDBOX_*` / OTLP environment variables, TOML `--config` file, then defaults.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::otel::SpanExporter;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("reading config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing config {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfigFile {
    pub server: ServerSection,
    pub scheduler: SchedulerSection,
    pub worker: WorkerSection,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSection {
    pub addr: Option<String>,
    pub data_dir: Option<String>,
    pub auth: Table,
    pub tls: Table,
    pub telemetry: TelemetrySection,
    pub scheduler: Table,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetrySection {
    pub metrics_addr: Option<String>,
    pub otlp_endpoint: Option<String>,
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerSection {
    pub addr: Option<String>,
    pub metadata: MetadataSection,
    pub cas: CasSection,
    pub pools: Table,
    pub idle: IdleSection,
    pub metering: Table,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MetadataSection {
    pub backend: Option<String>,
    pub dsn: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CasSection {
    pub backend: Option<String>,
    pub path: Option<String>,
    pub bucket: Option<String>,
    pub cache_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IdleSection {
    pub sleep_after_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WorkerSection {
    pub addr: Option<String>,
    pub tenant_id: Option<String>,
    pub scheduler_addr: Option<String>,
    pub isolation: Table,
    pub resources: Table,
    pub tier2: Table,
}

/// Accepts unknown-free nested tables used by infra schemas but not yet mapped
/// into server runtime behavior.
pub type Table = toml::Table;

/// Resolved serve settings. CLI flags are applied by the caller after [`load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServeConfig {
    pub addr: String,
    pub data_dir: Option<PathBuf>,
    pub gc_interval_secs: u64,
    pub idle_sleep_secs: u64,
    pub database_url: Option<String>,
    pub otel_endpoint: Option<String>,
    pub object_store_bucket: Option<String>,
    pub object_store_endpoint: Option<String>,
    /// Not loaded from the TOML schema; set by CLI `--python-artifacts` when provided.
    pub python_artifacts: Option<PathBuf>,
}

impl Default for ResolvedServeConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:8080".to_string(),
            data_dir: None,
            gc_interval_secs: 600,
            idle_sleep_secs: 300,
            database_url: None,
            otel_endpoint: None,
            object_store_bucket: None,
            object_store_endpoint: None,
            python_artifacts: None,
        }
    }
}

/// Loads config from `path`. A missing explicit `path` is an error; a missing
/// default path (`/etc/sandbox/config.toml` when `path` is `None`) is ignored.
pub fn load(path: Option<&Path>) -> Result<ResolvedServeConfig, ConfigError> {
    let mut resolved = ResolvedServeConfig::default();
    let explicit = path.is_some();
    let path = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/sandbox/config.toml"));
    if path.exists() {
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let file: ServerConfigFile =
            toml::from_str(&text).map_err(|source| ConfigError::Parse { path, source })?;
        apply_file(&mut resolved, &file);
    } else if explicit {
        return Err(ConfigError::Read {
            path,
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "config file not found"),
        });
    }
    apply_env(&mut resolved);
    Ok(resolved)
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}

fn nonempty(s: &Option<String>) -> Option<String> {
    s.as_ref()
        .and_then(|v| if v.is_empty() { None } else { Some(v.clone()) })
}

fn apply_file(resolved: &mut ResolvedServeConfig, file: &ServerConfigFile) {
    if let Some(addr) = &file.server.addr {
        resolved.addr = addr.clone();
    }
    if let Some(dir) = &file.server.data_dir {
        resolved.data_dir = Some(expand_tilde(dir));
    }
    if let Some(ep) = nonempty(&file.server.telemetry.otlp_endpoint) {
        resolved.otel_endpoint = Some(ep);
    }
    if let Some(secs) = file.scheduler.idle.sleep_after_seconds {
        resolved.idle_sleep_secs = secs;
    }
    if let Some(dsn) = nonempty(&file.scheduler.metadata.dsn) {
        resolved.database_url = Some(dsn);
    }
    if let Some(bucket) = nonempty(&file.scheduler.cas.bucket) {
        resolved.object_store_bucket = Some(bucket);
    }
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn apply_env(resolved: &mut ResolvedServeConfig) {
    if let Some(v) = env_str("SANDBOX_SERVER_ADDR") {
        resolved.addr = v;
    }
    if let Some(v) = env_str("SANDBOX_SERVER_DATA_DIR") {
        resolved.data_dir = Some(expand_tilde(&v));
    }
    if let Some(v) = env_str("OTEL_EXPORTER_OTLP_ENDPOINT") {
        resolved.otel_endpoint = Some(v);
    }
    if let Some(v) = env_str("SANDBOX_SERVER_TELEMETRY_OTLP_ENDPOINT") {
        resolved.otel_endpoint = Some(v);
    }
    if let Some(v) = env_str("SANDBOX_DATABASE_URL") {
        resolved.database_url = Some(v);
    }
    if let Some(v) = env_str("SANDBOX_SCHEDULER_METADATA_DSN") {
        resolved.database_url = Some(v);
    }
    if let Some(v) = env_str("SANDBOX_IDLE_SLEEP_SECS")
        .or_else(|| env_str("SANDBOX_SCHEDULER_IDLE_SLEEP_AFTER_SECONDS"))
    {
        if let Ok(n) = v.parse() {
            resolved.idle_sleep_secs = n;
        }
    }
    if let Some(v) = env_str("SANDBOX_OBJECT_STORE_BUCKET") {
        resolved.object_store_bucket = Some(v);
    }
    if let Some(v) = env_str("SANDBOX_OBJECT_STORE_ENDPOINT") {
        resolved.object_store_endpoint = Some(v);
    }
}

pub fn telemetry_from_resolved(resolved: &ResolvedServeConfig) -> Option<SpanExporter> {
    resolved.otel_endpoint.as_deref().map(SpanExporter::start)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infrastructure/config/examples/dev.toml");
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
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infrastructure/config/examples/dev.toml");
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
}
