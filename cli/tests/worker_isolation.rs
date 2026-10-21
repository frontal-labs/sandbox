//! Worker isolation on Linux (PLAN.md §4.13). Each test starts a real worker
//! process from the `sandbox` binary and checks, from outside, what the kernel
//! applied to it. Needs Linux with cgroup v2, namespaces, and seccomp, so the
//! whole file is compiled only there. Run it with `docker/run-linux.sh`.
#![cfg(target_os = "linux")]

use sandbox_types::{ErrorCode, ExecRequest};
use sandbox_worker::{CgroupLimits, Isolation, Worker};
use std::fs;

fn worker_path() -> String {
    std::env::var("CARGO_BIN_EXE_frontal-sandbox")
        .expect("Cargo should provide the frontal-sandbox test binary path")
}

async fn strict_worker() -> Option<Worker> {
    match Worker::spawn_with(&worker_path(), Isolation::strict()).await {
        Ok(worker) => Some(worker),
        Err(error) if error.code == ErrorCode::Unsupported => {
            eprintln!("skipping isolation test: {error}");
            None
        }
        Err(error) => panic!("a strict worker starts: {error}"),
    }
}

/// The value of one `Key:` line in `/proc/<pid>/status`.
fn status_field(pid: u32, key: &str) -> String {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}:")))
        .unwrap()
        .trim()
        .to_string()
}

/// The cgroup v2 directory of process `pid`.
fn cgroup_dir(pid: u32) -> std::path::PathBuf {
    let text = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap();
    let relative = text.lines().find_map(|l| l.strip_prefix("0::")).unwrap();
    std::path::Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'))
}

#[tokio::test]
async fn a_strict_worker_runs_with_no_new_privileges_and_a_seccomp_filter() {
    let Some(worker) = strict_worker().await else { return };
    let pid = worker.pid().unwrap();
    assert_eq!(status_field(pid, "NoNewPrivs"), "1");
    // Mode 2 is the filter mode. Mode 0 would mean the filter is absent.
    assert_eq!(status_field(pid, "Seccomp"), "2");
    worker.shutdown().await;
}

#[tokio::test]
async fn a_strict_worker_has_its_own_network_and_ipc_namespaces() {
    let Some(worker) = strict_worker().await else { return };
    let pid = worker.pid().unwrap();
    for ns in ["net", "ipc", "uts", "mnt"] {
        let theirs = fs::read_link(format!("/proc/{pid}/ns/{ns}")).unwrap();
        let ours = fs::read_link(format!("/proc/self/ns/{ns}")).unwrap();
        assert_ne!(theirs, ours, "the worker shares the host's {ns} namespace");
    }
    worker.shutdown().await;
}

#[tokio::test]
async fn a_strict_worker_is_in_a_cgroup_with_its_limits() {
    let limits = CgroupLimits::default();
    let Some(worker) = strict_worker().await else { return };
    let pid = worker.pid().unwrap();
    let dir = cgroup_dir(pid);
    assert_eq!(
        fs::read_to_string(dir.join("memory.max")).unwrap().trim(),
        limits.memory_bytes.to_string()
    );
    assert_eq!(
        fs::read_to_string(dir.join("pids.max")).unwrap().trim(),
        limits.max_pids.to_string()
    );
    assert_eq!(
        fs::read_to_string(dir.join("cpu.max")).unwrap().trim(),
        format!("{} {}", limits.cpu_quota_us, limits.cpu_period_us)
    );
    worker.shutdown().await;
}

#[tokio::test]
async fn a_strict_worker_still_runs_commands() {
    let Some(mut worker) = strict_worker().await else { return };
    let out = worker
        .exec(ExecRequest {
            argv: vec!["echo".to_string(), "isolated".to_string()],
            ..ExecRequest::default()
        })
        .await
        .expect("a command runs in the isolated worker");
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, b"isolated\n");
    worker.shutdown().await;
}

#[tokio::test]
async fn the_cgroup_is_removed_once_the_worker_is_dropped() {
    let Some(worker) = strict_worker().await else { return };
    let dir = cgroup_dir(worker.pid().unwrap());
    assert!(dir.exists());
    drop(worker);
    // The child is killed on drop and reaped by the kernel shortly after. Wait for
    // the cgroup to empty, then it can be removed.
    for _ in 0..100 {
        if !dir.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the worker's cgroup {} was not removed", dir.display());
}

#[tokio::test]
async fn an_unisolated_worker_reports_no_isolation() {
    let worker = Worker::spawn_with(&worker_path(), Isolation::none())
        .await
        .expect("an unisolated worker starts");
    assert!(worker.isolation().is_none());
    let pid = worker.pid().unwrap();
    // No filter and no privilege lock: the host's values, which are 0 for a plain process.
    assert_eq!(status_field(pid, "Seccomp"), "0");
    worker.shutdown().await;
}
