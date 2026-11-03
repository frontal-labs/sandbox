use sandbox_types::{SandboxError, SessionConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[cfg(feature = "firecracker")]
use firecracker::client::{FirecrackerClient, FirecrackerConfig};

#[derive(Debug, Clone)]
pub struct VmmDriver {
    #[cfg(feature = "firecracker")]
    client: Option<FirecrackerClient>,
    firecracker_binary: PathBuf,
    kernel_image: PathBuf,
    rootfs_image: PathBuf,
    #[cfg(not(feature = "firecracker"))]
    _phantom: std::marker::PhantomData<()>,
}

#[derive(Debug, Clone)]
pub struct VmHandle {
    pub id: String,
    pub vsock_addr: SocketAddr,
    pub pid: Option<u32>,
    pub rootfs: PathBuf,
}

impl VmmDriver {
    pub fn new() -> Self {
        // Try to find firecracker binary
        let firecracker_binary =
            which::which("firecracker").unwrap_or_else(|_| PathBuf::from("/usr/bin/firecracker"));

        // Default paths for kernel and rootfs (would be configured)
        let kernel_image = PathBuf::from("/opt/firecracker/vmlinux.bin");
        let rootfs_image = PathBuf::from("/opt/firecracker/rootfs.ext4");

        #[cfg(feature = "firecracker")]
        let client = None;

        Self {
            #[cfg(feature = "firecracker")]
            client,
            firecracker_binary,
            kernel_image,
            rootfs_image,
            #[cfg(not(feature = "firecracker"))]
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_paths(
        firecracker_binary: PathBuf,
        kernel_image: PathBuf,
        rootfs_image: PathBuf,
    ) -> Self {
        #[cfg(feature = "firecracker")]
        let client = None;

        Self {
            #[cfg(feature = "firecracker")]
            client,
            firecracker_binary,
            kernel_image,
            rootfs_image,
            #[cfg(not(feature = "firecracker"))]
            _phantom: std::marker::PhantomData,
        }
    }

    /// Starts a Firecracker VM.
    pub async fn start_vm(&mut self, config: &SessionConfig) -> Result<VmHandle, SandboxError> {
        #[cfg(feature = "firecracker")]
        {
            self.start_vm_firecracker(config).await
        }

        #[cfg(not(feature = "firecracker"))]
        {
            self.start_vm_spawn(config).await
        }
    }

    /// Starts VM using the firecracker crate (if available).
    #[cfg(feature = "firecracker")]
    async fn start_vm_firecracker(
        &mut self,
        config: &SessionConfig,
    ) -> Result<VmHandle, SandboxError> {
        // Create a unique socket path for this VM
        let vm_id = uuid::Uuid::new_v4().to_string();
        let socket_path = format!("/tmp/firecracker-{}.sock", vm_id);

        let fc_config = FirecrackerConfig::new(socket_path.clone());
        let client = FirecrackerClient::new(fc_config).await.map_err(|e| {
            SandboxError::internal(format!("Failed to connect to Firecracker: {}", e))
        })?;

        // Configure the VM
        let vm_config = firecracker::model::VmConfiguration {
            vcpu_count: config.limits.max_processes.unwrap_or(1) as u32,
            mem_size_mib: (config.limits.memory_bytes.unwrap_or(512 * 1024 * 1024) / (1024 * 1024))
                as u64,
            ht_enabled: false,
        };
        client
            .put_vm_configuration(&vm_config)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to set VM config: {}", e)))?;

        // Configure boot source
        let boot_source = firecracker::model::BootSource {
            kernel_image_path: self.kernel_image.to_string_lossy().to_string(),
            boot_args: Some("console=ttyS0 reboot=k panic=1 pci=off".to_string()),
            initrd_path: None,
        };
        client
            .put_boot_source(&boot_source)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to set boot source: {}", e)))?;

        // Configure rootfs
        let drive = firecracker::model::Drive {
            drive_id: "rootfs".to_string(),
            path_on_host: self.rootfs_image.to_string_lossy().to_string(),
            is_root_device: true,
            is_read_only: false,
            partuuid: None,
            cache_type: Some("Unsafe".to_string()),
            io_engine: Some("Sync".to_string()),
            rate_limiter: None,
        };
        client
            .put_drive("rootfs", &drive)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to set drive: {}", e)))?;

        // Configure network (vsock)
        let vsock = firecracker::model::VsockDevice {
            vsock_id: "vsock0".to_string(),
            guest_cid: 3,
            uds_path: format!("/tmp/firecracker-{}-vsock.sock", vm_id),
        };
        client
            .put_vsock_device(&vsock)
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to set vsock: {}", e)))?;

        // Start the VM
        client
            .put_actions(firecracker::model::ActionRequest {
                action_type: "InstanceStart".to_string(),
            })
            .await
            .map_err(|e| SandboxError::internal(format!("Failed to start VM: {}", e)))?;

        self.client = Some(client);

        Ok(VmHandle {
            id: vm_id,
            vsock_addr: SocketAddr::from(([127, 0, 0, 1], 0)), // Would be actual vsock address
            pid: None,
            rootfs: self.rootfs_image.clone(),
        })
    }

    /// Starts VM by spawning the firecracker binary directly.
    #[cfg(not(feature = "firecracker"))]
    async fn start_vm_spawn(&mut self, config: &SessionConfig) -> Result<VmHandle, SandboxError> {
        let vm_id = uuid::Uuid::new_v4().to_string();
        let socket_path = format!("/tmp/firecracker-{}.sock", vm_id);
        let vsock_path = format!("/tmp/firecracker-{}-vsock.sock", vm_id);

        // Clean up any existing socket
        let _ = tokio::fs::remove_file(&socket_path).await;
        let _ = tokio::fs::remove_file(&vsock_path).await;

        // Build firecracker command
        let mut cmd = Command::new(&self.firecracker_binary);
        cmd.arg("--api-sock")
            .arg(&socket_path)
            .arg("--kernel-image")
            .arg(&self.kernel_image)
            .arg("--rootfs")
            .arg(&self.rootfs_image)
            .arg("--kernel-args")
            .arg("console=ttyS0 reboot=k panic=1 pci=off")
            .arg("--cpus")
            .arg(config.limits.max_processes.unwrap_or(1).to_string())
            .arg("--memory")
            .arg(
                (config.limits.memory_bytes.unwrap_or(512 * 1024 * 1024) / (1024 * 1024))
                    .to_string(),
            )
            .arg("--vsock")
            .arg(format!("vsock0:guest_cid=3,uds_path={}", vsock_path))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| SandboxError::internal(format!("Failed to spawn firecracker: {}", e)))?;

        // Wait for socket to be ready
        let pid = child.id().unwrap_or(0);
        let mut retries = 50;
        while retries > 0 {
            if tokio::fs::metadata(&socket_path).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            retries -= 1;
        }

        if retries == 0 {
            let _ = child.kill().await;
            return Err(SandboxError::internal("Firecracker socket did not appear"));
        }

        // Spawn log readers
        if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout);
            tokio::spawn(async move {
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    tracing::debug!(target: "firecracker", "{}", line.trim_end());
                    line.clear();
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let mut reader = BufReader::new(stderr);
            tokio::spawn(async move {
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    tracing::warn!(target: "firecracker", "{}", line.trim_end());
                    line.clear();
                }
            });
        }

        Ok(VmHandle {
            id: vm_id,
            vsock_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            pid: Some(pid),
            rootfs: self.rootfs_image.clone(),
        })
    }

    /// Stops the VM.
    pub async fn stop_vm(&mut self) -> Result<(), SandboxError> {
        #[cfg(feature = "firecracker")]
        {
            if let Some(client) = &self.client {
                client
                    .put_actions(firecracker::model::ActionRequest {
                        action_type: "SendCtrlAltDel".to_string(),
                    })
                    .await
                    .map_err(|e| SandboxError::internal(format!("Failed to stop VM: {}", e)))?;
                self.client = None;
            }
        }

        #[cfg(not(feature = "firecracker"))]
        {
            // Would need to track the child process
        }

        Ok(())
    }

    /// Sends a command to the guest via vsock.
    pub async fn send_vsock_command(
        &self,
        _handle: &VmHandle,
        _command: &[u8],
    ) -> Result<Vec<u8>, SandboxError> {
        // Would implement vsock communication
        Err(SandboxError::unsupported(
            "vsock communication not implemented",
        ))
    }
}

impl Default for VmmDriver {
    fn default() -> Self {
        Self::new()
    }
}
