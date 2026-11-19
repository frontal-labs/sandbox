//! M4 acceptance: "promotion under 400 ms" (PLAN.md section 6). Times `promote_to_vm` for a
//! Tier 1 session, taken from a warm pool, over trees of growing size. The warm handout is what
//! a promotion waits for when the pool has a guest, and the tree copy is the rest of it.
//!
//! The 400 ms bound is asserted only for the small workspace, and only under hardware
//! virtualisation (KVM or HVF): the target describes hardware, and QEMU's software emulator is
//! not it. Every size is printed so the cost of a larger tree is on record. Ignored unless
//! SANDBOX_VM_IMAGE names a built guest image (docker/build-guest-image.sh builds it).

use sandbox_promote::promote_to_vm;
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{SessionConfig, TenantId};
use sandbox_vfs::OpenFlags;
use sandbox_vmm::{Accel, QemuSpec, VmPool};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Writes `files` files of `size` bytes each under `/work`, `per_dir` to a directory.
async fn populate(session: &dyn Session, files: usize, size: usize, per_dir: usize) {
    let fs = session.fs();
    fs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    let data = vec![b'x'; size];
    let flags = OpenFlags {
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    for i in 0..files {
        let dir = format!("/work/d{}", i / per_dir);
        if i % per_dir == 0 {
            fs.mkdir(Path::new(&dir), 0o755).await.unwrap();
        }
        let handle = fs
            .open(Path::new(&format!("{dir}/f{i}.bin")), flags)
            .await
            .unwrap();
        fs.write(&handle, 0, &data).await.unwrap();
        fs.close(&handle);
    }
}

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/build-guest-image.sh"]
async fn promotion_from_a_warm_pool_is_timed_across_workspace_sizes() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let accel = Accel::from_env();
    let pool = VmPool::new(
        QemuSpec {
            qemu: PathBuf::from("qemu-system-x86_64"),
            kernel: dir.join("vmlinuz"),
            initrd: dir.join("initrd.gz"),
            memory_mib: 2048,
            accel,
            console: dir.join("promotion-latency-console.log"),
        },
        1,
        Duration::from_secs(300),
    );

    // (label, files, bytes per file, files per directory). The first is a typical agent
    // workspace: a few source files. The others bound what the file-copy design costs.
    let shapes = [
        ("empty", 0usize, 0usize, 1usize),
        ("small_8x4KiB", 8, 4 * 1024, 8),
        ("medium_200x8KiB", 200, 8 * 1024, 50),
        ("large_1x32MiB", 1, 32 * 1024 * 1024, 1),
    ];
    let mut report = Vec::new();
    for (label, files, size, per_dir) in shapes {
        pool.refill().await.unwrap();
        let session = IsolateBackend::new(SessionConfig {
            tenant_id: TenantId::new(),
            ..SessionConfig::default()
        })
        .await
        .unwrap();
        if files > 0 {
            populate(&session, files, size, per_dir).await;
        }

        let started = Instant::now();
        let vm = pool.take().await.unwrap();
        let handout = started.elapsed();
        let (promoted, copied) = promote_to_vm(&session, vm).await.unwrap();
        let total = started.elapsed();
        report.push((label, total, handout, copied.files, copied.bytes));
        drop(promoted);
    }

    for (label, total, handout, files, bytes) in &report {
        println!(
            "PROMOTION (QEMU {}) {label}: total_ms={} warm_handout_ms={} files={files} bytes={bytes}",
            accel.name(),
            total.as_millis(),
            handout.as_millis(),
        );
    }

    if accel != Accel::Tcg {
        let small = report.iter().find(|r| r.0 == "small_8x4KiB").unwrap();
        assert!(
            small.1 < Duration::from_millis(400),
            "promotion of a small workspace took {:?}, target is 400 ms",
            small.1
        );
    }
}
