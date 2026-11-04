//! The warm pool boots guests ahead of time, hands them out, and boots on demand when empty.
//! Ignored unless SANDBOX_VM_IMAGE names a built guest image (docker/run-vm-tests.sh builds it).

use sandbox_vmm::{Accel, QemuSpec, VmPool};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
#[ignore = "needs SANDBOX_VM_IMAGE; see docker/run-vm-tests.sh"]
async fn the_pool_keeps_ready_guests_and_boots_on_demand() {
    let dir = PathBuf::from(std::env::var("SANDBOX_VM_IMAGE").expect("set SANDBOX_VM_IMAGE"));
    let pool = VmPool::new(
        QemuSpec {
            qemu: PathBuf::from("qemu-system-x86_64"),
            kernel: dir.join("vmlinuz"),
            initrd: dir.join("initrd.gz"),
            memory_mib: 2048,
            accel: Accel::from_env(),
            console: dir.join("pool-console.log"),
        },
        1,
        Duration::from_secs(180),
    );

    assert_eq!(pool.refill().await.unwrap(), 1);
    assert_eq!(pool.ready().await, 1);
    // The next promotion takes the warm guest: nothing is booted on its path.
    let booted_before = pool.booted();
    let warm = pool.take().await.unwrap();
    assert_eq!(
        pool.booted(),
        booted_before,
        "a warm guest must not boot again"
    );
    assert_eq!(pool.ready().await, 0);
    // A second take finds the pool empty and boots one.
    let cold = pool.take().await.unwrap();
    assert_eq!(pool.booted(), booted_before + 1);

    warm.stop().await.unwrap();
    cold.stop().await.unwrap();
}
