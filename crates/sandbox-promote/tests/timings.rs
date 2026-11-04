//! Times the Tier 2 steps under QEMU's software emulator: a cold boot, a warm handout from the
//! pool, and a promotion of a small tree. The numbers are printed, not asserted, because the
//! emulator is not the hardware the targets in PLAN.md section 6 describe. Ignored unless
//! SANDBOX_VM_IMAGE names a built guest image (docker/run-vm-tests.sh builds it).

use sandbox_cas::MemStore;
use sandbox_promote::promote_tree;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_vmm::{Accel, QemuSpec, VmPool};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn print_tier2_timings() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let pool = VmPool::new(
        QemuSpec {
            qemu: PathBuf::from("qemu-system-x86_64"),
            kernel: dir.join("vmlinuz"),
            initrd: dir.join("initrd.gz"),
            memory_mib: 2048,
            accel: Accel::from_env(),
            console: dir.join("timings-console.log"),
        },
        1,
        Duration::from_secs(300),
    );

    let started = Instant::now();
    pool.refill().await.unwrap();
    let cold_boot = started.elapsed();

    let vfs = Vfs::new(ContentHash::nil(), Arc::new(MemStore::new()));
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    let flags = OpenFlags {
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new("/work/small.txt"), flags).await.unwrap();
    vfs.write(&handle, 0, &vec![b'x'; 64 * 1024]).await.unwrap();
    vfs.close(&handle);

    // The warm handout is what a promotion waits for when the pool has a guest.
    let started = Instant::now();
    let vm = pool.take().await.unwrap();
    let warm_handout = started.elapsed();

    let mut guest = sandbox_proto::v1::guest::guest_service_client::GuestServiceClient::connect(
        format!("http://{}", vm.agent()),
    )
    .await
    .unwrap();
    let started = Instant::now();
    let report = promote_tree(&vfs, &mut guest).await.unwrap();
    let promotion = started.elapsed();

    println!(
        "TIER2 TIMINGS (QEMU TCG, this host): cold_boot_ms={} warm_handout_ms={} promote_64kib_ms={} files={} bytes={}",
        cold_boot.as_millis(),
        warm_handout.as_millis(),
        promotion.as_millis(),
        report.files,
        report.bytes
    );
    vm.stop().await.unwrap();
}
