//! A guest VM run by QEMU (PLAN.md section 4.10). Under Docker on hosts without KVM, QEMU
//! runs the guest in software (TCG), so the tier works without hardware virtualisation.
//! With KVM, the same arguments run the guest at hardware speed. The guest runs the guest
//! agent as its init, reachable on a port the host forwards.
//!
//! The VM is stopped when the value is dropped (`kill_on_drop`), so a session that ends
//! early cannot leave a QEMU process behind.

use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_types::{ErrorCode, SandboxError};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

/// Port the guest agent listens on inside the guest (see docker/emulated-vm/build-image.sh).
pub const GUEST_AGENT_PORT: u16 = 7000;

/// How QEMU runs the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accel {
    /// Software emulation. Works everywhere, and is slow.
    Tcg,
    /// Hardware virtualisation through /dev/kvm.
    Kvm,
}

impl Accel {
    /// The accelerator the environment names: `SANDBOX_VM_ACCEL=kvm` for hardware virtualisation,
    /// and software emulation otherwise. The emulated CI and the KVM runner set it.
    pub fn from_env() -> Accel {
        match std::env::var("SANDBOX_VM_ACCEL").as_deref() {
            Ok("kvm") => Accel::Kvm,
            _ => Accel::Tcg,
        }
    }

    fn arg(self) -> &'static str {
        match self {
            Accel::Tcg => "tcg",
            Accel::Kvm => "kvm",
        }
    }
}

/// Everything QEMU needs to boot one guest.
#[derive(Debug, Clone)]
pub struct QemuSpec {
    pub qemu: PathBuf,
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub memory_mib: u32,
    pub accel: Accel,
    /// Where the guest's serial console is written. Kept, so a failed boot can be read.
    pub console: PathBuf,
}

/// A running guest. Its agent answers on `agent`.
#[derive(Debug)]
pub struct QemuVm {
    child: Child,
    agent: SocketAddr,
}

impl QemuVm {
    /// Boots the guest and returns once QEMU is running. Call [`QemuVm::wait_for_agent`]
    /// before using it.
    pub async fn start(spec: &QemuSpec) -> Result<Self, SandboxError> {
        let port = free_local_port()?;
        let console = format!("file:{}", spec.console.display());
        let mut cmd = Command::new(&spec.qemu);
        cmd.args([
            "-accel",
            spec.accel.arg(),
            "-M",
            "q35",
            "-m",
            &spec.memory_mib.to_string(),
            "-smp",
            "1",
            "-no-reboot",
            "-display",
            "none",
            "-monitor",
            "none",
            "-kernel",
        ])
        .arg(&spec.kernel)
        .arg("-initrd")
        .arg(&spec.initrd)
        .args(["-append", "console=ttyS0 rdinit=/init panic=-1"])
        .arg("-netdev")
        .arg(format!(
            "user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:{GUEST_AGENT_PORT}"
        ))
        .args(["-device", "virtio-net-pci,netdev=net0", "-serial"])
        .arg(console)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        let child = cmd.spawn().map_err(|e| {
            SandboxError::new(
                ErrorCode::VmBootFailed,
                format!("cannot start {}: {e}", spec.qemu.display()),
            )
        })?;
        Ok(QemuVm {
            child,
            agent: SocketAddr::from(([127, 0, 0, 1], port)),
        })
    }

    /// The address of the guest agent, as the host reaches it.
    pub fn agent(&self) -> SocketAddr {
        self.agent
    }

    /// Waits until the agent answers a `Ping` over the guest protocol, or fails after
    /// `timeout`. QEMU accepts forwarded connections before the agent listens, so an open
    /// port is not enough. QEMU exiting first is reported at once, with the reason it gave.
    pub async fn wait_for_agent(&mut self, timeout: Duration) -> Result<(), SandboxError> {
        let deadline = Instant::now() + timeout;
        let url = format!("http://{}", self.agent);
        loop {
            if let Some(status) = self.child.try_wait().map_err(io_error)? {
                let stderr = self.stderr_text().await;
                return Err(SandboxError::new(
                    ErrorCode::VmBootFailed,
                    format!("QEMU exited with {status} before the agent answered: {stderr}"),
                ));
            }
            if let Ok(mut client) = GuestServiceClient::connect(url.clone()).await {
                if client.ping(()).await.is_ok() {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(SandboxError::new(
                    ErrorCode::VmBootFailed,
                    format!("the guest agent did not answer within {timeout:?}"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Stops the guest and waits for QEMU to exit.
    pub async fn stop(mut self) -> Result<(), SandboxError> {
        let _ = self.child.kill().await;
        self.child.wait().await.map_err(io_error)?;
        Ok(())
    }

    async fn stderr_text(&mut self) -> String {
        use tokio::io::AsyncReadExt;
        let mut text = String::new();
        if let Some(stderr) = self.child.stderr.as_mut() {
            let _ = stderr.read_to_string(&mut text).await;
        }
        text.trim().chars().take(500).collect()
    }
}

/// A local port that was free when asked. Another process could take it before QEMU binds
/// it. The caller then sees the agent fail to answer, and the boot is retried.
fn free_local_port() -> Result<u16, SandboxError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
    let port = listener.local_addr().map_err(io_error)?.port();
    drop(listener);
    Ok(port)
}

fn io_error(error: std::io::Error) -> SandboxError {
    SandboxError::new(ErrorCode::VmBootFailed, error.to_string())
}
