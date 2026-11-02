use sandbox_types::{SandboxError, SessionConfig};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

pub struct Jailer {
    jailer_binary: PathBuf,
}

impl Jailer {
    pub fn new() -> Self {
        let jailer_binary =
            which::which("jailer").unwrap_or_else(|_| PathBuf::from("/usr/bin/jailer"));
        Self { jailer_binary }
    }

    pub fn with_binary(jailer_binary: PathBuf) -> Self {
        Self { jailer_binary }
    }

    /// Sets up the Firecracker jailer.
    /// This creates the chroot environment and necessary directories for Firecracker.
    pub async fn setup(&self, config: &SessionConfig) -> Result<JailerConfig, SandboxError> {
        let vm_id = uuid::Uuid::new_v4().to_string();
        let base_dir = PathBuf::from("/run/firecracker").join(&vm_id);

        // Create the jailer directory structure
        let rootfs_dir = base_dir.join("rootfs");
        let kernel_dir = base_dir.join("kernel");
        let config_dir = base_dir.join("config");

        tokio::fs::create_dir_all(&rootfs_dir)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to create rootfs dir: {}", e)))?;
        tokio::fs::create_dir_all(&kernel_dir)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to create kernel dir: {}", e)))?;
        tokio::fs::create_dir_all(&config_dir)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to create config dir: {}", e)))?;

        // Build jailer command
        let mut cmd = Command::new(&self.jailer_binary);
        cmd.arg("--id")
            .arg(&vm_id)
            .arg("--exec-file")
            .arg("/usr/bin/firecracker")
            .arg("--chroot-base-dir")
            .arg("/run/firecracker")
            .arg("--num-cpus")
            .arg(config.limits.max_processes.unwrap_or(1).to_string())
            .arg("--memory")
            .arg(
                (config.limits.memory_bytes.unwrap_or(512 * 1024 * 1024) / (1024 * 1024))
                    .to_string(),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to run jailer: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SandboxError::internal(format!("Jailer failed: {}", stderr)));
        }

        Ok(JailerConfig {
            vm_id,
            base_dir,
            rootfs_dir,
            kernel_dir,
            config_dir,
            chroot_base: PathBuf::from("/run/firecracker"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct JailerConfig {
    pub vm_id: String,
    pub base_dir: PathBuf,
    pub rootfs_dir: PathBuf,
    pub kernel_dir: PathBuf,
    pub config_dir: PathBuf,
    pub chroot_base: PathBuf,
}

impl Default for Jailer {
    fn default() -> Self {
        Self::new()
    }
}
