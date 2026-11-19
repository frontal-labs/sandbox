//! An in-memory server for load tests: `sandbox serve`, with several API keys and a choice of
//! runtime. `sandbox serve` creates one key and runs on a single-thread runtime, and one key
//! may hold 64 sessions (`MAX_SESSIONS_PER_KEY`), so it cannot be loaded past 64 sessions.
//!
//! ```text
//! cargo run --release -p sandbox-server --example load_node -- \
//!     --addr 127.0.0.1:18080 --keys 20 --keys-file keys.txt --worker-threads 0
//! ```
//!
//! `--worker-threads 0` uses the current-thread runtime that `sandbox serve` uses. The keys are
//! written to `--keys-file`, comma separated, for `SANDBOX_API_KEYS`.

use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut addr = "127.0.0.1:18080".to_string();
    let mut keys = 1usize;
    let mut keys_file = None;
    let mut worker_threads = 0usize;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--addr" => addr = value()?,
            "--keys" => keys = value()?.parse()?,
            "--keys-file" => keys_file = Some(value()?),
            "--worker-threads" => worker_threads = value()?.parse()?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }

    let auth = Arc::new(sandbox_server::auth::AuthManager::new());
    let created: Vec<String> = (0..keys).map(|_| auth.create_key()).collect();
    if let Some(path) = keys_file {
        std::fs::write(path, created.join(","))?;
    }
    let scheduler = Arc::new(sandbox_scheduler::Scheduler::new());

    let mut builder = if worker_threads == 0 {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut b = tokio::runtime::Builder::new_multi_thread();
        b.worker_threads(worker_threads);
        b
    };
    let rt = builder.enable_all().build()?;
    rt.block_on(async move {
        let (app, state) = sandbox_server::create_router_with_options(
            scheduler,
            auth,
            sandbox_server::ServerOptions::default(),
        );
        // The same background sweeps `sandbox serve` runs by default.
        sandbox_server::gc::schedule(state.clone(), Duration::from_secs(600));
        sandbox_server::idle::schedule(state, Duration::from_secs(300), Duration::from_secs(30));
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        eprintln!(
            "load_node listening on {addr}: {keys} key(s), {}",
            if worker_threads == 0 {
                "current-thread runtime".to_string()
            } else {
                format!("{worker_threads} worker threads")
            }
        );
        axum::serve(listener, app).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
