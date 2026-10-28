//! `sandbox bench`: times a fixed set of operations on the local isolate runtime,
//! the same code paths `run` and `shell` use. Results are JSON lines, one per
//! case, so a script can compare runs.

use sandbox_session::{restore, IsolateBackend, Session};
use sandbox_types::{ErrorCode, SandboxError, SessionConfig};
use sandbox_vfs::OpenFlags;
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant};

/// Cases in the order they run. Each name is also a valid `--name`.
pub const CASES: &[&str] = &[
    "exec_echo",
    "shell_pipeline",
    "vfs_write_read_4k",
    "snapshot_restore",
    "session_create",
    "node_script",
];

const WARMUP: usize = 10;
const ITERATIONS: usize = 200;
const FILE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaseResult {
    pub name: String,
    pub iterations: usize,
    pub mean_ns: u64,
    pub min_ns: u64,
    pub p50_ns: u64,
    pub p90_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// Summarizes per-iteration times. Percentiles are exact nearest-rank values.
pub fn summarize(name: &str, samples: &mut [Duration]) -> CaseResult {
    samples.sort_unstable();
    let n = samples.len();
    let ns = |d: Duration| d.as_nanos() as u64;
    let rank = |p: f64| {
        let index = (p / 100.0 * n as f64).ceil() as usize;
        ns(samples[index.clamp(1, n) - 1])
    };
    let total: u128 = samples.iter().map(|d| d.as_nanos()).sum();
    CaseResult {
        name: name.to_string(),
        iterations: n,
        mean_ns: (total / n as u128) as u64,
        min_ns: ns(samples[0]),
        p50_ns: rank(50.0),
        p90_ns: rank(90.0),
        p99_ns: rank(99.0),
        max_ns: ns(samples[n - 1]),
    }
}

/// Runs the cases named by `name`, or all of them, and prints one JSON line each.
pub async fn run(name: Option<&str>) -> Result<i32, SandboxError> {
    let selected: Vec<&str> = match name {
        Some(wanted) => {
            if !CASES.contains(&wanted) {
                return Err(SandboxError::new(
                    ErrorCode::InvalidArgument,
                    format!("unknown benchmark {wanted}; known: {}", CASES.join(", ")),
                ));
            }
            vec![wanted]
        }
        None => CASES.to_vec(),
    };
    for case in selected {
        let result = measure(case).await?;
        let line = serde_json::to_string(&result)
            .map_err(|e| SandboxError::internal(format!("cannot encode result: {e}")))?;
        println!("{line}");
    }
    Ok(0)
}

async fn measure(case: &str) -> Result<CaseResult, SandboxError> {
    let mut samples = Vec::with_capacity(ITERATIONS);
    let session = IsolateBackend::new(SessionConfig::default()).await?;
    let payload = vec![b'x'; FILE_BYTES];
    for i in 0..(WARMUP + ITERATIONS) {
        let started = Instant::now();
        match case {
            "exec_echo" => {
                session.run_script("echo bench").await?;
            }
            "shell_pipeline" => {
                session.run_script("echo a | cat | cat").await?;
            }
            "vfs_write_read_4k" => {
                write_then_read(&session, &payload).await?;
            }
            "snapshot_restore" => {
                let root = session.snapshot().await?;
                restore(root).await?;
            }
            "session_create" => {
                // The cold start of a Tier 1 session: a new backend, with nothing warm.
                let fresh = IsolateBackend::new(SessionConfig::default()).await?;
                drop(fresh);
            }
            "node_script" => {
                // A `node` run: a new V8 isolate per script, which is the cost a warm
                // isolate pool would remove.
                sandbox_runtime::execute_script("1 + 1", sandbox_runtime::ScriptLimits::default())
                    .await?;
            }
            other => {
                return Err(SandboxError::internal(format!(
                    "no benchmark named {other}"
                )));
            }
        }
        if i >= WARMUP {
            samples.push(started.elapsed());
        }
    }
    Ok(summarize(case, &mut samples))
}

/// Writes the payload to one file and reads it back, through the session's VFS.
async fn write_then_read(session: &dyn Session, payload: &[u8]) -> Result<(), SandboxError> {
    let path = Path::new("/bench.dat");
    let fs = session.fs();
    let write = OpenFlags {
        read: true,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = fs.open(path, write).await.map_err(vfs_failure)?;
    fs.write(&handle, 0, payload).await.map_err(vfs_failure)?;
    fs.close(&handle);

    let handle = fs
        .open(path, OpenFlags::default())
        .await
        .map_err(vfs_failure)?;
    let read = fs
        .read(&handle, 0, payload.len())
        .await
        .map_err(vfs_failure)?;
    fs.close(&handle);
    if read.len() != payload.len() {
        return Err(SandboxError::internal("read back a different length"));
    }
    Ok(())
}

fn vfs_failure(error: sandbox_vfs::VfsError) -> SandboxError {
    SandboxError::new(ErrorCode::VfsError, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_exact_nearest_rank_values() {
        let mut samples: Vec<Duration> = (1..=100u64).rev().map(Duration::from_nanos).collect();
        let r = summarize("case", &mut samples);
        assert_eq!(r.iterations, 100);
        assert_eq!((r.min_ns, r.max_ns), (1, 100));
        assert_eq!((r.p50_ns, r.p90_ns, r.p99_ns), (50, 90, 99));
        assert_eq!(r.mean_ns, 50);
    }

    #[test]
    fn a_single_sample_is_every_percentile() {
        let r = summarize("one", &mut [Duration::from_nanos(7)]);
        assert_eq!((r.p50_ns, r.p99_ns, r.mean_ns), (7, 7, 7));
    }

    #[tokio::test]
    async fn an_unknown_case_is_refused_and_lists_the_known_ones() {
        let err = run(Some("nope")).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("exec_echo"));
    }
}
