//! Verifies the minimal [server]-only TOML shape used by the Kubernetes
//! ConfigMap (infrastructure/kubernetes/manifests.yaml) parses cleanly. The
//! loader must not require [scheduler] or [worker] top-level tables, because
//! the K8s manifest only ships [server] + secrets injected via env.

use sandbox_server::config::load;
use std::io::Write;
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

/// The exact [server] block emitted by the K8s ConfigMap (api_key_hash empty;
/// it is overridden by the SANDBOX_API_KEY_HASH env at runtime).
const K8S_CONFIG_TOML: &str = r#"
[server]
addr = "0.0.0.0:8080"
data_dir = "/var/lib/sandbox"

[server.auth]
api_key_hash = ""
rate_limit_rps = 100

[server.tls]
enabled = false

[server.telemetry]
metrics_addr = "0.0.0.0:9090"
otlp_endpoint = "http://otel-collector.monitoring.svc:4317"
service_name = "sandbox-server"
"#;

#[test]
fn k8s_configmap_shape_loads() {
    let _g = env_lock().lock().expect("lock");
    clear_env();

    let mut dir = std::env::temp_dir();
    dir.push(format!("sandbox-k8s-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(K8S_CONFIG_TOML.as_bytes()).unwrap();
    drop(f);

    let resolved = load(Some(&path)).expect("K8s ConfigMap shape must parse");

    // Clean up.
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);

    assert_eq!(resolved.addr, "0.0.0.0:8080");
    assert_eq!(
        resolved.data_dir.as_deref(),
        Some(std::path::Path::new("/var/lib/sandbox"))
    );
    assert_eq!(
        resolved.otel_endpoint.as_deref(),
        Some("http://otel-collector.monitoring.svc:4317")
    );
}
