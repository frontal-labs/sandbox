//! M4 acceptance: "ffmpeg and headless Chromium run via auto-promotion". An `auto` session starts
//! in the isolate tier, and the first command the tier cannot run (`ffmpeg`) moves it to a guest
//! over the HTTP API. The files the session already held come along, ffmpeg encodes a clip in
//! the guest, and headless Chromium renders a page and a screenshot there. Every step goes
//! through `POST /v1/sessions/{id}/exec`, as an agent's would. Ignored unless SANDBOX_VM_IMAGE
//! names a guest image that carries both programs (docker/build-guest-image.sh builds it).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::{create_router_with_vm, VmConfig};
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
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

/// Runs `argv` in the session, and returns the exit code and the text of stdout and stderr.
async fn exec(
    app: &axum::Router,
    key: &str,
    id: &str,
    argv: &[&str],
) -> (i64, String, String, Duration) {
    let started = Instant::now();
    let (status, body) = call(
        app,
        key,
        "POST",
        &format!("/v1/sessions/{id}/exec"),
        Some(json!({"argv": argv, "env": {}, "stream": false, "timeout_ms": 120_000})),
    )
    .await;
    let took = started.elapsed();
    assert_eq!(status, StatusCode::OK, "{argv:?}: {body}");
    let text = |field: &str| {
        let bytes: Vec<u8> = body[field]
            .as_array()
            .map(|a| a.iter().map(|b| b.as_u64().unwrap() as u8).collect())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (
        body["exit_code"].as_i64().unwrap(),
        text("stdout"),
        text("stderr"),
        took,
    )
}

async fn tier(app: &axum::Router, key: &str, id: &str) -> String {
    let (status, info) = call(app, key, "GET", &format!("/v1/sessions/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    info["tier"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE with ffmpeg and chromium; see docker/build-guest-image.sh"]
async fn ffmpeg_and_headless_chromium_run_through_auto_promotion() {
    let image = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let auth = Arc::new(AuthManager::new());
    let key = auth.create_key();
    let accel = sandbox_vmm::Accel::from_env();
    let (app, _state) = create_router_with_vm(
        Arc::new(Scheduler::new()),
        auth,
        VmConfig {
            qemu: PathBuf::from("qemu-system-x86_64"),
            image,
            accel,
            memory_mib: 2048,
            boot_timeout: Duration::from_secs(300),
            warm_guests: 1,
        },
    );
    // The pool boots its guest in the background. Give it the time a warm server would have had.
    tokio::time::sleep(Duration::from_secs(
        std::env::var("SANDBOX_WARM_WAIT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20),
    ))
    .await;

    let config = serde_json::to_value(SessionConfig {
        tenant_id: TenantId::nil(),
        tier_preference: TierPreference::Auto,
        ..SessionConfig::default()
    })
    .unwrap();
    let (status, created) = call(&app, &key, "POST", "/v1/sessions", Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["session_id"].as_str().unwrap().to_string();

    // Tier 1 work first: the session holds a file, and stays in the isolate tier.
    let (code, _, err, _) = exec(
        &app,
        &key,
        &id,
        &[
            "sh",
            "-c",
            "mkdir -p /work && echo carried > /work/marker.txt",
        ],
    )
    .await;
    assert_eq!(code, 0, "{err}");
    assert_eq!(tier(&app, &key, &id).await, "isolate");

    // ffmpeg is not an isolate tool: this command promotes the session, then runs in the guest.
    let (code, _, err, promoting) = exec(
        &app,
        &key,
        &id,
        &[
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=128x96:rate=10",
            "-pix_fmt",
            "yuv420p",
            "/work/clip.mp4",
        ],
    )
    .await;
    assert_eq!(code, 0, "ffmpeg encode: {err}");
    assert_eq!(tier(&app, &key, &id).await, "vm");

    // The file from Tier 1 came across, and ffmpeg's output is in the same tree.
    let (code, out, err, _) = exec(&app, &key, &id, &["cat", "/work/marker.txt"]).await;
    assert_eq!((code, out.as_str()), (0, "carried\n"), "{err}");
    // Decoding the clip proves the encode wrote a valid file into the session's tree.
    let (code, _, err, _) = exec(
        &app,
        &key,
        &id,
        &[
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "/work/clip.mp4",
            "-f",
            "null",
            "-",
        ],
    )
    .await;
    assert_eq!(code, 0, "ffmpeg decode of its own output: {err}");

    // Headless Chromium renders a page, and writes a screenshot into the session's tree.
    let chromium_flags = [
        "chromium",
        "--headless",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
    ];
    let mut dump = chromium_flags.to_vec();
    dump.extend(["--dump-dom", "data:text/html,<p>auto-promoted</p>"]);
    let (code, out, err, chromium_dump) = exec(&app, &key, &id, &dump).await;
    assert_eq!(code, 0, "chromium --dump-dom: {err}");
    assert!(out.contains("<p>auto-promoted</p>"), "{out}");

    let mut shot = chromium_flags.to_vec();
    shot.extend([
        "--screenshot=/work/shot.png",
        "data:text/html,<h1>shot</h1>",
    ]);
    let (code, _, err, _) = exec(&app, &key, &id, &shot).await;
    assert_eq!(code, 0, "chromium --screenshot: {err}");
    let (code, out, err, _) = exec(&app, &key, &id, &["sh", "-c", "wc -c < /work/shot.png"]).await;
    assert_eq!(code, 0, "{err}");
    assert!(
        out.trim().parse::<u64>().unwrap_or(0) > 100,
        "screenshot is not empty: {out:?}"
    );

    println!(
        "AUTOPROMOTE TIMINGS (QEMU {}): first_command_including_promotion_ms={} chromium_dump_dom_ms={}",
        accel.name(),
        promoting.as_millis(),
        chromium_dump.as_millis()
    );
}
