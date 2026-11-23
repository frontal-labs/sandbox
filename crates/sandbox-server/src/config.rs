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
    pub tls: TlsSection,
    pub telemetry: TelemetrySection,
    pub scheduler: Table,
    pub rate_limit: Option<RateLimitSection>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitSection {
    /// Maximum requests per key per second. `0` disables limiting.
    pub requests_per_second: Option<u64>,
}

/// Parsed contents of the `[server.tls]` table.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TlsSection {
    pub enabled: bool,
    pub cert: Option<String>,
    pub key: Option<String>,
    /// CA bundle used to verify client certificates (mTLS).
    pub ca: Option<String>,
    pub require_client_auth: bool,
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
///
/// `Debug` is written by hand so the database password in `database_url` never
/// reaches a log line or a panic message.
#[derive(Clone, PartialEq, Eq)]
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
    /// Maximum live sessions the server holds across all keys.
    pub max_sessions_total: usize,
    /// Maximum snapshots the server holds across all keys.
    pub max_snapshots_total: usize,
    /// Maximum requests per key per second. `0` disables limiting.
    pub rate_limit_rps: Option<u64>,
    /// TLS termination settings for the HTTP listener.
    pub tls: crate::tls::TlsConfig,
}

impl std::fmt::Debug for ResolvedServeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedServeConfig")
            .field("addr", &self.addr)
            .field("data_dir", &self.data_dir)
            .field("gc_interval_secs", &self.gc_interval_secs)
            .field("idle_sleep_secs", &self.idle_sleep_secs)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "<redacted>"),
            )
            .field("otel_endpoint", &self.otel_endpoint)
            .field("object_store_bucket", &self.object_store_bucket)
            .field("object_store_endpoint", &self.object_store_endpoint)
            .field("python_artifacts", &self.python_artifacts)
            .field("max_sessions_total", &self.max_sessions_total)
            .field("max_snapshots_total", &self.max_snapshots_total)
            .field("rate_limit_rps", &self.rate_limit_rps)
            .field("tls", &self.tls)
            .finish()
    }
}

/// Security settings the file asks for that the server does not implement.
pub fn ignored_security_settings(file: &ServerConfigFile) -> Vec<String> {
    let mut out = Vec::new();
    if !file.server.auth.is_empty() {
        out.push(
            "[server.auth] settings (such as api_key_hash) are not \
             implemented and are ignored."
                .to_string(),
        );
    }
    out
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
            max_sessions_total: 20_000,
            max_snapshots_total: 20_000,
            rate_limit_rps: None,
            tls: crate::tls::TlsConfig::default(),
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
        for warning in ignored_security_settings(&file) {
            tracing::warn!("{warning}");
        }
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

pub fn apply_file(resolved: &mut ResolvedServeConfig, file: &ServerConfigFile) {
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
    if let Some(rps) = file
        .server
        .rate_limit
        .as_ref()
        .and_then(|r| r.requests_per_second)
    {
        resolved.rate_limit_rps = Some(rps);
    }
    resolved.tls = crate::tls::TlsConfig {
        enabled: file.server.tls.enabled,
        cert: file.server.tls.cert.as_deref().map(expand_tilde),
        key: file.server.tls.key.as_deref().map(expand_tilde),
        ca: file.server.tls.ca.as_deref().map(expand_tilde),
        require_client_auth: file.server.tls.require_client_auth,
    };
}



pub fn env_str(key: &str) -> Option<String> {
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
    if let Some(v) = env_str("SANDBOX_MAX_SESSIONS_TOTAL") {
        if let Ok(n) = v.parse() {
            resolved.max_sessions_total = n;
        }
    }
    if let Some(v) = env_str("SANDBOX_MAX_SNAPSHOTS_TOTAL") {
        if let Ok(n) = v.parse() {
            resolved.max_snapshots_total = n;
        }
    }
    if let Some(v) = env_str("SANDBOX_RATE_LIMIT_RPS") {
        if let Ok(n) = v.parse() {
            resolved.rate_limit_rps = Some(n);
        }
    }
    if let Some(v) = env_str("SANDBOX_TLS_CERT") {
        resolved.tls.cert = Some(expand_tilde(&v));
    }
    if let Some(v) = env_str("SANDBOX_TLS_KEY") {
        resolved.tls.key = Some(expand_tilde(&v));
    }
    if let Some(v) = env_str("SANDBOX_TLS_CA") {
        resolved.tls.ca = Some(expand_tilde(&v));
    }
    if let Some(v) = env_str("SANDBOX_TLS_REQUIRE_CLIENT_AUTH") {
        resolved.tls.require_client_auth = v == "true" || v == "1";
    }
}

pub fn telemetry_from_resolved(resolved: &ResolvedServeConfig) -> Option<SpanExporter> {
    resolved.otel_endpoint.as_deref().map(SpanExporter::start)
}
