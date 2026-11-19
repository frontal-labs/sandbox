//! A guest VM run by QEMU (PLAN.md section 4.10). Under Docker on hosts without KVM, QEMU
//! runs the guest in software (TCG), so the tier works without hardware virtualisation.
//! With KVM, the same arguments run the guest at hardware speed. The guest runs the guest
//! agent as its init, reachable on a port the host forwards.
//!
//! The guest is x86_64 by default. On Apple Silicon the same driver runs an aarch64 guest
//! under HVF (`SANDBOX_VM_ARCH=aarch64 SANDBOX_VM_ACCEL=hvf`), because HVF cannot run an
//! x86_64 guest on an arm64 host and emulating one is far too slow for the Tier 2 targets.
//!
//! The VM is stopped when the value is dropped (`kill_on_drop`), so a session that ends
//! early cannot leave a QEMU process behind.

use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_types::{ErrorCode, SandboxError};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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
    /// Hardware virtualisation through Apple's Hypervisor.framework. The guest must match the
    /// host's architecture, so on Apple Silicon it is an aarch64 guest.
    Hvf,
}

impl Accel {
    /// The accelerator the environment names: `SANDBOX_VM_ACCEL=kvm` or `hvf` for hardware
    /// virtualisation, and software emulation otherwise. The emulated CI, the KVM runner, and
    /// an Apple Silicon host set it.
    pub fn from_env() -> Accel {
        match std::env::var("SANDBOX_VM_ACCEL").as_deref() {
            Ok("kvm") => Accel::Kvm,
            Ok("hvf") => Accel::Hvf,
            _ => Accel::Tcg,
        }
    }

    /// The accelerator's name, for reports.
    pub fn name(self) -> &'static str {
        self.arg()
    }

    fn arg(self) -> &'static str {
        match self {
            Accel::Tcg => "tcg",
            Accel::Kvm => "kvm",
            Accel::Hvf => "hvf",
        }
    }

    fn is_hardware(self) -> bool {
        !matches!(self, Accel::Tcg)
    }
}

/// The CPU architecture of the guest. It fixes the QEMU binary, the machine, and the serial
/// console the kernel writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestArch {
    X86_64,
    Aarch64,
}

impl GuestArch {
    /// The guest's architecture: `SANDBOX_VM_ARCH` (`x86_64`, or `aarch64`/`arm64`) when set,
    /// else the one `qemu`'s name carries, else x86_64.
    pub fn detect(qemu: &Path) -> GuestArch {
        match std::env::var("SANDBOX_VM_ARCH").as_deref() {
            Ok("aarch64" | "arm64") => return GuestArch::Aarch64,
            Ok("x86_64" | "amd64") => return GuestArch::X86_64,
            _ => {}
        }
        let name = qemu.file_name().map(|n| n.to_string_lossy().into_owned());
        match name {
            Some(n) if n.contains("aarch64") => GuestArch::Aarch64,
            _ => GuestArch::X86_64,
        }
    }

    fn binary_name(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "qemu-system-x86_64",
            GuestArch::Aarch64 => "qemu-system-aarch64",
        }
    }

    fn console(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "ttyS0",
            GuestArch::Aarch64 => "ttyAMA0",
        }
    }

    /// The machine and CPU arguments. A hardware accelerator runs the host's own CPU on aarch64.
    fn machine_args(self, accel: Accel) -> Vec<&'static str> {
        match self {
            GuestArch::X86_64 => vec!["-M", "q35"],
            GuestArch::Aarch64 if accel.is_hardware() => vec!["-M", "virt", "-cpu", "host"],
            GuestArch::Aarch64 => vec!["-M", "virt", "-cpu", "max"],
        }
    }

    /// The QEMU binary to run. A caller that names the x86_64 binary (the tests do) gets the
    /// aarch64 one beside it when the guest is aarch64, so one setting picks the architecture.
    fn resolve(self, qemu: &Path) -> PathBuf {
        let is_default_x86 = qemu.file_name().is_some_and(|n| n == "qemu-system-x86_64");
        if self == GuestArch::Aarch64 && is_default_x86 {
            return qemu.with_file_name(self.binary_name());
        }
        qemu.to_path_buf()
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

/// Where a guest's state is saved, and resumed from: a qcow2 disk that holds the saved state, and
/// the QEMU monitor socket that `save_state` talks to. A VM started with it can be saved, and a VM
/// started with `resume` set comes back in the state that was saved under that tag.
#[derive(Debug, Clone)]
pub struct Snapshotting {
    pub disk: PathBuf,
    pub monitor: PathBuf,
    /// A saved state to resume, by its tag, instead of booting the guest.
    pub resume: Option<String>,
}

/// A running guest. Its agent answers on `agent`.
#[derive(Debug)]
pub struct QemuVm {
    child: Child,
    agent: SocketAddr,
    monitor: Option<PathBuf>,
}

impl QemuVm {
    /// Boots the guest and returns once QEMU is running. Call [`QemuVm::wait_for_agent`]
    /// before using it.
    pub async fn start(spec: &QemuSpec) -> Result<Self, SandboxError> {
        Self::start_with(spec, None).await
    }

    /// Like [`QemuVm::start`], and the guest can be saved with [`QemuVm::save_state`], or resumed
    /// from a saved state when `snapshotting.resume` is set.
    pub async fn start_with(
        spec: &QemuSpec,
        snapshotting: Option<&Snapshotting>,
    ) -> Result<Self, SandboxError> {
        let port = free_local_port()?;
        let console = format!("file:{}", spec.console.display());
        let arch = GuestArch::detect(&spec.qemu);
        let mut cmd = Command::new(arch.resolve(&spec.qemu));
        cmd.args(["-accel", spec.accel.arg()])
            .args(arch.machine_args(spec.accel))
            .args([
                "-m",
                &spec.memory_mib.to_string(),
                "-smp",
                "1",
                "-no-reboot",
                "-display",
                "none",
            ])
            .args(monitor_args(snapshotting))
            .arg("-kernel")
            .arg(&spec.kernel)
            .arg("-initrd")
            .arg(&spec.initrd)
            .args([
                "-append",
                &format!("console={} rdinit=/init panic=-1", arch.console()),
            ])
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
            monitor: snapshotting.map(|s| s.monitor.clone()),
        })
    }

    /// Saves the guest's state under `tag`, on the disk `start_with` was given. QEMU pauses the
    /// guest while it saves, and the guest runs again afterwards. Call it before [`QemuVm::stop`].
    pub async fn save_state(&self, tag: &str) -> Result<(), SandboxError> {
        let monitor = self.monitor.as_ref().ok_or_else(|| {
            SandboxError::new(
                ErrorCode::InvalidState,
                "this VM was started without a saved-state disk",
            )
        })?;
        let reply = monitor_command(monitor, &format!("savevm {tag}")).await?;
        if reply.to_ascii_lowercase().contains("error") {
            return Err(SandboxError::new(
                ErrorCode::VmBootFailed,
                format!("saving the state failed: {}", reply.trim()),
            ));
        }
        Ok(())
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

/// The QEMU arguments for the monitor, and the saved-state disk when there is one. Without it the
/// monitor is off, as it always was.
fn monitor_args(snapshotting: Option<&Snapshotting>) -> Vec<String> {
    let Some(s) = snapshotting else {
        return vec!["-monitor".into(), "none".into()];
    };
    let mut args = vec![
        "-chardev".into(),
        format!(
            "socket,id=mon0,path={},server=on,wait=off",
            s.monitor.display()
        ),
        "-mon".into(),
        "chardev=mon0,mode=readline".into(),
        "-drive".into(),
        format!("file={},format=qcow2,if=none,id=vmstate", s.disk.display()),
        "-device".into(),
        "virtio-blk-pci,drive=vmstate".into(),
    ];
    if let Some(tag) = &s.resume {
        args.push("-loadvm".into());
        args.push(tag.clone());
    }
    args
}

/// Sends one line to QEMU's human monitor on `socket`, and returns what it printed. The socket
/// appears when QEMU starts, so the connection is retried for a while.
async fn monitor_command(socket: &Path, line: &str) -> Result<String, SandboxError> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match tokio::net::UnixStream::connect(socket).await {
            Ok(stream) => return send_line(stream, line).await,
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(SandboxError::new(
                        ErrorCode::VmBootFailed,
                        format!("cannot reach the QEMU monitor at {}: {e}", socket.display()),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Waits for the monitor's first prompt, sends the line, and returns the reply.
async fn send_line(mut stream: tokio::net::UnixStream, line: &str) -> Result<String, SandboxError> {
    use tokio::io::AsyncWriteExt;
    read_to_prompt(&mut stream).await?;
    stream
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(io_error)?;
    read_to_prompt(&mut stream).await
}

/// Reads from the monitor until its prompt, and returns the text before it. A save can take as
/// long as the guest's memory takes to write, so the wait is generous.
async fn read_to_prompt(stream: &mut tokio::net::UnixStream) -> Result<String, SandboxError> {
    use tokio::io::AsyncReadExt;
    const PROMPT: &[u8] = b"(qemu) ";
    let mut text = Vec::new();
    let mut chunk = [0u8; 4096];
    while !text.ends_with(PROMPT) {
        let read = tokio::time::timeout(Duration::from_secs(300), stream.read(&mut chunk))
            .await
            .map_err(|_| {
                SandboxError::new(ErrorCode::VmBootFailed, "the QEMU monitor did not answer")
            })?
            .map_err(io_error)?;
        if read == 0 {
            // The monitor closed: what it said so far is the reply.
            return Ok(String::from_utf8_lossy(&text).into_owned());
        }
        text.extend_from_slice(&chunk[..read]);
    }
    let end = text.len().saturating_sub(PROMPT.len());
    let text = if text.ends_with(PROMPT) {
        &text[..end]
    } else {
        &text[..]
    };
    Ok(String::from_utf8_lossy(text).into_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_x86_guest_keeps_its_q35_machine_and_serial_console() {
        let arch = GuestArch::X86_64;
        assert_eq!(arch.machine_args(Accel::Tcg), ["-M", "q35"]);
        assert_eq!(arch.machine_args(Accel::Kvm), ["-M", "q35"]);
        assert_eq!(arch.console(), "ttyS0");
    }

    #[test]
    fn an_aarch64_guest_runs_the_host_cpu_only_under_hardware_virtualisation() {
        let arch = GuestArch::Aarch64;
        assert_eq!(
            arch.machine_args(Accel::Hvf),
            ["-M", "virt", "-cpu", "host"]
        );
        assert_eq!(arch.machine_args(Accel::Tcg), ["-M", "virt", "-cpu", "max"]);
        assert_eq!(arch.console(), "ttyAMA0");
    }

    #[test]
    fn an_aarch64_guest_swaps_the_default_x86_binary_for_the_aarch64_one() {
        let x86 = Path::new("/opt/homebrew/bin/qemu-system-x86_64");
        assert_eq!(
            GuestArch::Aarch64.resolve(x86),
            Path::new("/opt/homebrew/bin/qemu-system-aarch64")
        );
        assert_eq!(GuestArch::X86_64.resolve(x86), x86);
        let custom = Path::new("/usr/local/bin/my-qemu");
        assert_eq!(GuestArch::Aarch64.resolve(custom), custom);
    }

    #[test]
    fn the_accelerator_names_map_to_qemu_arguments() {
        assert_eq!(Accel::Tcg.name(), "tcg");
        assert_eq!(Accel::Kvm.name(), "kvm");
        assert_eq!(Accel::Hvf.name(), "hvf");
        assert!(Accel::Hvf.is_hardware() && !Accel::Tcg.is_hardware());
    }
}
