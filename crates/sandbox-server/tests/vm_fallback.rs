//! A command the isolate tier cannot run moves an `auto` session to a Tier 2 guest, and the
//! session reports `vm` afterwards. Ignored unless SANDBOX_VM_IMAGE names a built guest image
//! (docker/run-vm-tests.sh builds it).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router_with_vm, VmConfig};
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn an_auto_session_moves_to_the_vm_when_a_command_needs_it() {
    let image = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let (app, _state) = create_router_with_vm(
        Arc::new(Scheduler::new()),
        auth,
        VmConfig {
            qemu: PathBuf::from("qemu-system-x86_64"),
            image,
            accel: sandbox_vmm::Accel::Tcg,
            memory_mib: 512,
            boot_timeout: Duration::from_secs(180),
        },
    );

    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Auto,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["session_id"].as_str().unwrap().to_string();
    call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["sh", "-c", "mkdir -p /work && echo kept > /work/f"], "env": {}, "stream": false})),
    )
    .await;

    let (status, body) = call(
        &app,
        &key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": ["nonexistent-tool"], "env": {}, "stream": false})),
    )
    .await;
    // The command ran in the guest, which has no such tool, so the guest's shell says so.
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exit_code"], 127, "{body}");

    let (status, info) = call(&app, &key, "GET", &format!("/v1/sessions/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["tier"], "vm", "{info}");
}
