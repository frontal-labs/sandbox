//! Memory of sleeping sessions, measured from outside the server process (M5 acceptance:
//! "10,000 idle sessions per host").
//!
//! `capacity.rs` measures in-process with explicit `/sleep` calls. This example differs in
//! three ways: the server is its own process, whose RSS an outside `ps` reads; sessions are put
//! to sleep by the real idle sweep (`idle::sweep` / `idle::schedule`), not by `/sleep`; and
//! sessions are spread over many keys and tenants, so the per-key and per-tenant limits do not
//! stop the run before the per-server limits do.
//!
//! ```text
//! idle_10k serve <data-dir> <addr> <keys-file> <keys>   # durable server, mints <keys> keys
//! idle_10k drive <addr> <keys-file> <server-pid> <n> <payload-bytes> <mode> <data-dir>
//! ```
//!
//! `serve` exposes two extra routes, for the harness only: `POST /_idle/sweep?after_ms=N` runs
//! one `idle::sweep` and returns how many sessions slept and how long it took, and
//! `POST /_idle/schedule?after_ms=N&every_ms=M` starts `idle::schedule`.
//!
//! `drive` modes: `bulk` creates every session, then runs one sweep. `trickle` starts the
//! schedule first and creates sessions while it runs, so they sleep as they go. Output is one
//! `key=value` line per stage.

use axum::extract::{Query, State};
use axum::routing::post;
use axum::{Json, Router};
use sandbox_scheduler::Scheduler;
use sandbox_server::{idle, AppState};
use sandbox_types::{SessionConfig, TenantId, TierPreference};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sessions one key holds. The stock limits are 64 live sessions and 32 snapshots per key, and
/// a sleep takes a snapshot, so 30 stays under both.
const SESSIONS_PER_KEY: usize = 30;
/// Sessions one tenant holds. The stock limit is 256.
const SESSIONS_PER_TENANT: usize = 240;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]).await,
        Some("drive") => drive(&args[1..]).await,
        _ => panic!("usage: idle_10k serve|drive ..."),
    }
}

async fn serve(args: &[String]) {
    let dir = PathBuf::from(&args[0]);
    let addr = &args[1];
    let keys_file = &args[2];
    let keys: usize = args[3].parse().unwrap();
    let (app, state, first) =
        sandbox_server::open_durable_with_state(Arc::new(Scheduler::new()), &dir)
            .await
            .expect("durable store opens");
    let mut minted: Vec<String> = first.into_iter().collect();
    while minted.len() < keys {
        minted.push(state.auth.create_key());
    }
    tokio::task::spawn_blocking({
        let keys_file = keys_file.to_string();
        let data = minted.join("\n");
        move || std::fs::write(&keys_file, data).unwrap()
    })
    .await
    .unwrap();
    let extra = Router::new()
        .route("/_idle/sweep", post(sweep_route))
        .route("/_idle/schedule", post(schedule_route))
        .with_state(state);
    let app = app.merge(extra);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    eprintln!("idle_10k server listening on {addr}, {} keys", minted.len());
    axum::serve(listener, app).await.unwrap();
}

async fn sweep_route(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, u64>>,
) -> Json<Value> {
    let after = Duration::from_millis(q.get("after_ms").copied().unwrap_or(0));
    let started = Instant::now();
    let slept = idle::sweep(&state, after).await;
    Json(json!({"slept": slept, "ms": started.elapsed().as_millis() as u64}))
}

async fn schedule_route(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, u64>>,
) -> Json<Value> {
    let after = Duration::from_millis(q.get("after_ms").copied().unwrap_or(2000));
    let every = Duration::from_millis(q.get("every_ms").copied().unwrap_or(1000));
    tokio::spawn(idle::schedule(state, after, every));
    Json(json!({"scheduled": true}))
}

/// Resident set size and virtual size of a process, in KiB, from `ps`.
fn ps_kib(pid: u32) -> (u64, u64) {
    let out = Command::new("ps")
        .args(["-o", "rss=,vsz=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text
        .split_whitespace()
        .map(|v| v.parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Physical footprint in KiB from macOS `footprint`, which counts compressed pages that `rss`
/// leaves out. 0 when the tool is not available.
fn footprint_kib(pid: u32) -> u64 {
    let Ok(out) = Command::new("footprint")
        .args(["-p", &pid.to_string()])
        .output()
    else {
        return 0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("Footprint:") {
            return parse_size_kib(rest.trim());
        }
    }
    0
}

fn parse_size_kib(text: &str) -> u64 {
    let mut parts = text.split_whitespace();
    let number: f64 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0.0);
    match parts.next().unwrap_or("KB") {
        "GB" => (number * 1024.0 * 1024.0) as u64,
        "MB" => (number * 1024.0) as u64,
        "KB" => number as u64,
        "bytes" | "B" => (number / 1024.0) as u64,
        _ => 0,
    }
}

fn open_fds(pid: u32) -> usize {
    Command::new("lsof")
        .args(["-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0)
}

fn dir_stats(dir: &Path) -> (u64, u64) {
    fn walk(dir: &Path, files: &mut u64, bytes: &mut u64) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), files, bytes);
            } else {
                *files += 1;
                *bytes += meta.len();
            }
        }
    }
    let (mut files, mut bytes) = (0, 0);
    walk(dir, &mut files, &mut bytes);
    (files, bytes)
}

fn stage(name: &str, pid: u32, dir: &Path, n: usize, extra: &str) {
    let (rss, vsz) = ps_kib(pid);
    let foot = footprint_kib(pid);
    let (files, bytes) = dir_stats(dir);
    println!(
        "stage={name} n={n} rss_kib={rss} vsz_kib={vsz} footprint_kib={foot} fds={} data_files={files} data_bytes={bytes} {extra}",
        open_fds(pid)
    );
}

async fn metric(client: &reqwest::Client, base: &str, key: &str, name: &str) -> u64 {
    let text = client
        .get(format!("{base}/metrics"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    text.lines()
        .find_map(|l| l.strip_prefix(name).and_then(|r| r.trim().parse().ok()))
        .unwrap_or(0)
}

async fn drive(args: &[String]) {
    let addr = &args[0];
    let keys_path = args[1].clone();
    let keys: Vec<String> = tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&keys_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();
    let pid: u32 = args[2].parse().unwrap();
    let n: usize = args[3].parse().unwrap();
    let payload: usize = args[4].parse().unwrap();
    let mode = args[5].as_str();
    let dir = PathBuf::from(&args[6]);
    let base = format!("http://{addr}");
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(32)
        .build()
        .unwrap();
    assert!(
        keys.len() * SESSIONS_PER_KEY >= n,
        "{} keys cover {} sessions, need {n}",
        keys.len(),
        keys.len() * SESSIONS_PER_KEY
    );
    let tenants: Vec<TenantId> = (0..n.div_ceil(SESSIONS_PER_TENANT))
        .map(|_| TenantId::new())
        .collect();

    stage("baseline", pid, &dir, 0, "");
    if mode == "trickle" {
        let r = client
            .post(format!("{base}/_idle/schedule?after_ms=2000&every_ms=1000"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
    }

    // Create every session with bounded concurrency. Each gets a unique payload file, so the
    // snapshot holds a blob of its own and nothing is shared between sessions.
    let started = Instant::now();
    let next = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(AtomicUsize::new(0));
    let first_error: Arc<std::sync::Mutex<Option<String>>> = Arc::new(Default::default());
    let ids: Arc<std::sync::Mutex<Vec<(usize, String)>>> = Arc::new(Default::default());
    let worker_count: usize = std::env::var("IDLE_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);
    let mut workers = Vec::new();
    for _ in 0..worker_count {
        let (client, base, keys, tenants) =
            (client.clone(), base.clone(), keys.clone(), tenants.clone());
        let (next, failures, first_error) = (next.clone(), failures.clone(), first_error.clone());
        let ids = ids.clone();
        workers.push(tokio::spawn(async move {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let key = &keys[i / SESSIONS_PER_KEY];
                let config = serde_json::to_value(SessionConfig {
                    tenant_id: tenants[i / SESSIONS_PER_TENANT],
                    tier_preference: TierPreference::Isolate,
                    ..SessionConfig::default()
                })
                .unwrap();
                let resp = client
                    .post(format!("{base}/v1/sessions"))
                    .bearer_auth(key)
                    .json(&config)
                    .send()
                    .await
                    .unwrap();
                let status = resp.status();
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                if !status.is_success() {
                    failures.fetch_add(1, Ordering::Relaxed);
                    first_error
                        .lock()
                        .unwrap()
                        .get_or_insert(format!("create {i}: {status} {body}"));
                    continue;
                }
                ids.lock()
                    .unwrap()
                    .push((i, body["session_id"].as_str().unwrap().to_string()));
                if payload > 0 {
                    let id = body["session_id"].as_str().unwrap();
                    let content = format!("{i:08}").repeat(payload / 8 + 1);
                    let resp = client
                        .put(format!("{base}/v1/sessions/{id}/fs/payload.bin"))
                        .bearer_auth(key)
                        .body(content.as_bytes()[..payload].to_vec())
                        .send()
                        .await
                        .unwrap();
                    if !resp.status().is_success() {
                        failures.fetch_add(1, Ordering::Relaxed);
                        let s = resp.status();
                        first_error
                            .lock()
                            .unwrap()
                            .get_or_insert(format!("write {i}: {s}"));
                    }
                }
            }
        }));
    }
    for w in workers {
        w.await.unwrap();
    }
    let created_ms = started.elapsed().as_millis();
    let failed = failures.load(Ordering::Relaxed);
    let live = n - failed;
    println!(
        "created={live} failed={failed} create_ms={created_ms} first_error={:?}",
        first_error.lock().unwrap()
    );
    stage(
        "after_create",
        pid,
        &dir,
        live,
        &format!("create_ms={created_ms}"),
    );

    // Put them to sleep.
    match mode {
        "bulk" => {
            let started = Instant::now();
            let r: Value = client
                .post(format!("{base}/_idle/sweep?after_ms=0"))
                .timeout(Duration::from_secs(4 * 3600))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            println!("sweep={r} wall_ms={}", started.elapsed().as_millis());
        }
        "trickle" => {
            let deadline = Instant::now() + Duration::from_secs(1800);
            while Instant::now() < deadline {
                let slept =
                    metric(&client, &base, &keys[0], "sandbox_sessions_slept_total").await as usize;
                if slept >= live {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        _ => panic!("mode is bulk or trickle"),
    }
    // The exec traffic of the create phase counts as activity, so wait a beat for the
    // sessions to go quiet before reading the counters.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let slept = metric(&client, &base, &keys[0], "sandbox_sessions_slept_total").await;
    let created = metric(&client, &base, &keys[0], "sandbox_sessions_created_total").await;
    println!("metrics slept={slept} created={created}");
    stage("after_sleep", pid, &dir, live, &format!("slept={slept}"));
    // Let the allocator and the page cache settle, and read again.
    tokio::time::sleep(Duration::from_secs(15)).await;
    stage("settled", pid, &dir, live, &format!("slept={slept}"));

    // How many of them are really asleep: GET a sample of up to 300.
    let ids = ids.lock().unwrap().clone();
    let step = (ids.len() / 300).max(1);
    let (mut checked, mut asleep) = (0, 0);
    for (i, id) in ids.iter().step_by(step) {
        let v: Value = client
            .get(format!("{base}/v1/sessions/{id}"))
            .bearer_auth(&keys[i / SESSIONS_PER_KEY])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        checked += 1;
        if v["status"] == "sleeping" {
            asleep += 1;
        }
    }
    println!("sample_checked={checked} sample_sleeping={asleep}");

    // Wake three sessions with a command that reads the payload back, and time each wake.
    if payload > 0 {
        for (i, id) in ids.iter().step_by((ids.len() / 3).max(1)).take(3) {
            let started = Instant::now();
            let v: Value = client
                .post(format!("{base}/v1/sessions/{id}/exec"))
                .bearer_auth(&keys[i / SESSIONS_PER_KEY])
                .json(&json!({"argv": ["wc", "-c", "payload.bin"], "env": {}, "cwd": null, "stdin": null, "timeout_ms": null, "stream": false}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            println!(
                "wake_exec session_index={i} ms={} exit={} stdout={}",
                started.elapsed().as_millis(),
                v["exit_code"],
                String::from_utf8_lossy(
                    &v["stdout"]
                        .as_array()
                        .map(|a| a
                            .iter()
                            .filter_map(|b| b.as_u64().map(|b| b as u8))
                            .collect::<Vec<u8>>())
                        .unwrap_or_default()
                )
                .trim()
            );
        }
        stage("after_wake3", pid, &dir, live, "");
    }
}
