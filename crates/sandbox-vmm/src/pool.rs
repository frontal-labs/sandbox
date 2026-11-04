//! A warm pool of guests (PLAN.md section 4.11). Each guest is booted, and its agent has
//! answered a `Ping`, before it is handed out. Promotion takes one from the pool, so the
//! boot is off the request's path. An empty pool boots one on demand, so a promotion never
//! waits for a refill.
//!
//! Guests in the pool hold no session state, so they are interchangeable. Each has its own
//! console log, named by a counter, so logs from concurrent boots do not overwrite each other.

use crate::qemu::{QemuSpec, QemuVm};
use sandbox_types::SandboxError;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct VmPool {
    spec: QemuSpec,
    boot_timeout: Duration,
    target: usize,
    ready: Mutex<VecDeque<QemuVm>>,
    booted: AtomicU64,
}

impl VmPool {
    /// A pool that keeps `target` guests ready, each started from `spec`.
    pub fn new(spec: QemuSpec, target: usize, boot_timeout: Duration) -> Arc<Self> {
        Arc::new(VmPool {
            spec,
            boot_timeout,
            target,
            ready: Mutex::new(VecDeque::new()),
            booted: AtomicU64::new(0),
        })
    }

    /// The number of guests ready to hand out.
    pub async fn ready(&self) -> usize {
        self.ready.lock().await.len()
    }

    /// The number of guests booted since the pool was made, ready or handed out.
    pub fn booted(&self) -> u64 {
        self.booted.load(Ordering::Relaxed)
    }

    /// Boots guests until the pool holds its target. Returns how many were booted.
    pub async fn refill(&self) -> Result<usize, SandboxError> {
        let mut booted = 0;
        while self.ready().await < self.target {
            let vm = self.boot().await?;
            self.ready.lock().await.push_back(vm);
            booted += 1;
        }
        Ok(booted)
    }

    /// Hands out a ready guest, or boots one if the pool is empty.
    pub async fn take(&self) -> Result<QemuVm, SandboxError> {
        let ready = self.ready.lock().await.pop_front();
        match ready {
            Some(vm) => Ok(vm),
            None => self.boot().await,
        }
    }

    async fn boot(&self) -> Result<QemuVm, SandboxError> {
        let n = self.booted.fetch_add(1, Ordering::Relaxed);
        let mut spec = self.spec.clone();
        spec.console = numbered(&self.spec.console, n);
        let mut vm = QemuVm::start(&spec).await?;
        vm.wait_for_agent(self.boot_timeout).await?;
        Ok(vm)
    }
}

/// `console.log` becomes `console-3.log`, so each boot has its own file.
fn numbered(path: &std::path::Path, n: u64) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "console".to_string());
    let name = match path.extension() {
        Some(ext) => format!("{stem}-{n}.{}", ext.to_string_lossy()),
        None => format!("{stem}-{n}"),
    };
    path.with_file_name(name)
}
