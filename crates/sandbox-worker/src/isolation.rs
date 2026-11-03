//! Linux isolation for worker processes (PLAN.md §4.13).
//!
//! A worker runs the isolate backend, which executes untrusted scripts. This
//! module limits what that process can reach, in three layers:
//!
//! - **Namespaces.** The child unshares its network, IPC, UTS, and mount
//!   namespaces, so it has no network interfaces and cannot see or change the
//!   host's mounts, hostname, or IPC objects. Its mounts are made private first,
//!   so nothing it does propagates to the host.
//! - **Seccomp.** A BPF filter denies syscalls that a script host never needs and
//!   that are the usual escape routes: debugging and cross-process memory access,
//!   kernel module and boot control, BPF and perf, keyrings, and mount and
//!   namespace manipulation. Denied calls fail with `EPERM`. `NO_NEW_PRIVS` is set
//!   by the filter install, so no later exec can gain privileges.
//! - **cgroups v2.** The host creates a cgroup for the worker with memory, pid,
//!   and CPU limits, and moves the worker into it right after it starts.
//!
//! Each layer fails closed. If one cannot be applied, the worker does not start,
//! so an unisolated worker is never served as if it were isolated.
//!
//! The filter is a denylist, so an unknown syscall is allowed. An allowlist is
//! stronger, and it is the next step once the set of syscalls the isolate needs
//! has been measured.

use sandbox_types::{ErrorCode, SandboxError};

/// The isolation a worker process gets. [`Isolation::strict`] is what a Linux
/// host should use. [`Isolation::none`] exists for platforms without these
/// mechanisms and for tests that check the rest of the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Isolation {
    /// Unshare the network, IPC, UTS, and mount namespaces.
    pub namespaces: bool,
    /// Install the seccomp denylist.
    pub seccomp: bool,
    /// Put the worker in a cgroup with these limits. `None` leaves it in the host's cgroup.
    pub cgroup: Option<CgroupLimits>,
}

/// Resource limits for a worker's cgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgroupLimits {
    /// Hard memory limit in bytes. The kernel kills the worker when it is exceeded.
    pub memory_bytes: u64,
    /// Most processes and threads the worker may hold.
    pub max_pids: u32,
    /// CPU time the worker may use per `cpu_period_us`, so `quota / period` is its CPU share.
    pub cpu_quota_us: u64,
    pub cpu_period_us: u64,
}

impl Default for CgroupLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 512 * 1024 * 1024,
            max_pids: 64,
            cpu_quota_us: 100_000,
            cpu_period_us: 100_000,
        }
    }
}

impl Isolation {
    /// Every layer on, with the default cgroup limits.
    pub fn strict() -> Self {
        Self {
            namespaces: true,
            seccomp: true,
            cgroup: Some(CgroupLimits::default()),
        }
    }

    /// No layer. A worker with this isolation is not a security boundary.
    pub fn none() -> Self {
        Self {
            namespaces: false,
            seccomp: false,
            cgroup: None,
        }
    }

    /// Whether every layer is off.
    pub fn is_none(&self) -> bool {
        *self == Self::none()
    }
}

impl Default for Isolation {
    /// Strict on Linux, and none elsewhere, where the layers do not exist.
    fn default() -> Self {
        if cfg!(target_os = "linux") {
            Self::strict()
        } else {
            Self::none()
        }
    }
}

/// Applies the in-process layers (namespaces and seccomp) to the calling process.
/// The worker calls this before it starts any thread, because a seccomp filter
/// applies only to the calling thread and to threads it creates later.
#[cfg(target_os = "linux")]
pub fn enter(isolation: &Isolation) -> Result<(), SandboxError> {
    if isolation.namespaces {
        unshare_namespaces()?;
    }
    if isolation.seccomp {
        install_seccomp()?;
    }
    Ok(())
}

/// Applies the in-process layers. Off Linux only a worker with no isolation can
/// start, because the layers do not exist there.
#[cfg(not(target_os = "linux"))]
pub fn enter(isolation: &Isolation) -> Result<(), SandboxError> {
    if isolation.is_none() {
        return Ok(());
    }
    Err(SandboxError::new(
        ErrorCode::Unsupported,
        "worker isolation needs Linux namespaces, seccomp, and cgroups",
    ))
}

#[cfg(target_os = "linux")]
fn os_error(what: &str) -> SandboxError {
    SandboxError::new(
        ErrorCode::Internal,
        format!("{what}: {}", std::io::Error::last_os_error()),
    )
}

#[cfg(target_os = "linux")]
fn unshare_namespaces() -> Result<(), SandboxError> {
    let flags = libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS | libc::CLONE_NEWNS;
    // SAFETY: `unshare` takes only flags and touches no memory of ours. It only
    // changes which namespaces this process belongs to.
    if unsafe { libc::unshare(flags) } != 0 {
        return Err(os_error("unsharing namespaces"));
    }
    // Stop mount events from propagating back to the host. The string arguments
    // are null, which `mount` accepts when only the propagation flags are set.
    let root = c"/";
    // SAFETY: `root` is a valid NUL-terminated string that lives for the call, and
    // the other pointer arguments are null, as propagation changes require.
    let rc = unsafe {
        libc::mount(
            std::ptr::null(),
            root.as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(os_error("making mounts private"));
    }
    Ok(())
}

/// Syscalls the worker denies. Each one is a way to reach another process, the
/// kernel, the host's mounts or namespaces, or privileged devices. None is needed
/// to run a script.
#[cfg(target_os = "linux")]
fn denied_syscalls() -> Vec<i64> {
    let denied = vec![
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_setns,
        libc::SYS_unshare,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
        libc::SYS_open_by_handle_at,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_acct,
        libc::SYS_quotactl,
        libc::SYS_fanotify_init,
    ];
    #[cfg(target_arch = "x86_64")]
    let denied = [denied, vec![libc::SYS_iopl, libc::SYS_ioperm]].concat();
    denied
}

/// Syscalls the worker explicitly allows. This is the allowlist approach,
/// which is more secure than the denylist because it only permits known-safe
/// syscalls. The list below includes syscalls needed for basic script execution,
/// file I/O, and memory management.
/// See the Linux syscall table for the architecture-specific numbers.
#[cfg(target_os = "linux")]
#[expect(dead_code)]
fn allowed_syscalls() -> Vec<i64> {
    let mut allowed = vec![
        // Process management
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        // Memory management
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mprotect,
        libc::SYS_brk,
        libc::SYS_madvise,
        libc::SYS_mincore,
        // File I/O
        libc::SYS_openat,
        libc::SYS_close,
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_lseek,
        libc::SYS_fstat,
        libc::SYS_statx,
        libc::SYS_fcntl,
        libc::SYS_ioctl,
        libc::SYS_dup,
        libc::SYS_dup2,
        libc::SYS_dup3,
        libc::SYS_pipe2,
        // Directory operations
        libc::SYS_getdents64,
        libc::SYS_mkdirat,
        libc::SYS_unlinkat,
        libc::SYS_renameat,
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_readlinkat,
        libc::SYS_fchdir,
        libc::SYS_getcwd,
        // Time
        libc::SYS_clock_gettime,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        // Signal handling
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        // Futex (for threading)
        libc::SYS_futex,
        // Socket (for vsock)
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_setsockopt,
        libc::SYS_getsockopt,
        libc::SYS_shutdown,
        // Event polling
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        // Miscellaneous
        libc::SYS_getrandom,
        libc::SYS_prlimit64,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_prctl,
        libc::SYS_arch_prctl,
        libc::SYS_sysinfo,
        libc::SYS_uname,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_yield,
        libc::SYS_sched_getparam,
        libc::SYS_sched_setparam,
        libc::SYS_sched_getscheduler,
        libc::SYS_sched_setscheduler,
        libc::SYS_sched_get_priority_max,
        libc::SYS_sched_get_priority_min,
    ];

    #[cfg(target_arch = "x86_64")]
    {
        allowed.extend([libc::SYS_ioctl, libc::SYS_sigaltstack, libc::SYS_arch_prctl]);
    }

    #[cfg(target_arch = "aarch64")]
    {
        allowed.extend([libc::SYS_ioctl, libc::SYS_sigaltstack]);
    }

    allowed.sort();
    allowed.dedup();
    allowed
}

/// Installs an allowlist-based seccomp filter (more secure than denylist).
#[cfg(target_os = "linux")]
#[expect(dead_code)]
fn install_seccomp_allowlist() -> Result<(), SandboxError> {
    use std::convert::TryInto;

    let rules = allowed_syscalls()
        .into_iter()
        .map(|nr| (nr, Vec::new()))
        .collect();
    // Allowed calls pass through, everything else fails with EPERM.
    let filter = seccompiler::SeccompFilter::new(
        rules,
        seccompiler::SeccompAction::Errno(libc::EPERM as u32),
        seccompiler::SeccompAction::Allow,
        target_arch()?,
    )
    .map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("building seccomp allowlist filter: {e}"),
        )
    })?;
    let program: seccompiler::BpfProgram = filter.try_into().map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("compiling seccomp allowlist filter: {e}"),
        )
    })?;
    seccompiler::apply_filter(&program).map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("installing seccomp allowlist filter: {e}"),
        )
    })
}

#[cfg(target_os = "linux")]
fn target_arch() -> Result<seccompiler::TargetArch, SandboxError> {
    #[cfg(target_arch = "x86_64")]
    return Ok(seccompiler::TargetArch::x86_64);
    #[cfg(target_arch = "aarch64")]
    return Ok(seccompiler::TargetArch::aarch64);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    Err(SandboxError::new(
        ErrorCode::Unsupported,
        "no seccomp target for this architecture",
    ))
}

#[cfg(target_os = "linux")]
fn install_seccomp() -> Result<(), SandboxError> {
    use std::convert::TryInto;

    let rules = denied_syscalls()
        .into_iter()
        .map(|nr| (nr, Vec::new()))
        .collect();
    // Denied calls fail with EPERM, and everything else is allowed.
    let filter = seccompiler::SeccompFilter::new(
        rules,
        seccompiler::SeccompAction::Allow,
        seccompiler::SeccompAction::Errno(libc::EPERM as u32),
        target_arch()?,
    )
    .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("building seccomp filter: {e}")))?;
    let program: seccompiler::BpfProgram = filter.try_into().map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("compiling seccomp filter: {e}"),
        )
    })?;
    // `apply_filter` sets `NO_NEW_PRIVS` before it installs the filter.
    seccompiler::apply_filter(&program).map_err(|e| {
        SandboxError::new(
            ErrorCode::Internal,
            format!("installing seccomp filter: {e}"),
        )
    })
}

/// The host's cgroup for one worker. Removed on drop, once the worker has exited
/// and the kernel has emptied it.
#[cfg(target_os = "linux")]
pub(crate) struct CgroupGuard {
    path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl CgroupGuard {
    /// Creates a cgroup named `name` under the host's own cgroup, with `limits`.
    /// Fails if the controllers are not delegated to it, so the limits are never
    /// silently absent.
    pub(crate) fn create(name: &str, limits: CgroupLimits) -> Result<Self, SandboxError> {
        let path = delegated_root()?.join(name);
        std::fs::create_dir(&path)
            .map_err(|e| SandboxError::internal(format!("creating worker cgroup: {e}")))?;
        let guard = Self { path };
        let writes = [
            ("memory.max", limits.memory_bytes.to_string()),
            // Without this the kernel swaps the excess out instead of killing the
            // process, so the limit would not hold. A host with swap would then let a
            // script run past its memory budget.
            ("memory.swap.max", "0".to_string()),
            ("pids.max", limits.max_pids.to_string()),
            (
                "cpu.max",
                format!("{} {}", limits.cpu_quota_us, limits.cpu_period_us),
            ),
        ];
        for (file, value) in writes {
            // A failed write drops `guard`, which removes the cgroup again.
            std::fs::write(guard.path.join(file), value).map_err(|e| {
                SandboxError::internal(format!("setting {file} on the worker cgroup: {e}"))
            })?;
        }
        Ok(guard)
    }

    /// Moves `pid` into the cgroup, so the limits apply to it.
    pub(crate) fn add(&self, pid: u32) -> Result<(), SandboxError> {
        std::fs::write(self.path.join("cgroup.procs"), pid.to_string())
            .map_err(|e| SandboxError::internal(format!("moving worker into its cgroup: {e}")))
    }

    /// How many processes the kernel has OOM-killed in this cgroup. Zero if the
    /// count cannot be read, so a caller never reports a kill it cannot confirm.
    pub(crate) fn oom_kills(&self) -> u64 {
        std::fs::read_to_string(self.path.join("memory.events"))
            .ok()
            .and_then(|events| {
                events
                    .lines()
                    .find_map(|line| line.strip_prefix("oom_kill "))
                    .and_then(|count| count.trim().parse().ok())
            })
            .unwrap_or(0)
    }
}

#[cfg(target_os = "linux")]
impl Drop for CgroupGuard {
    fn drop(&mut self) {
        // The kernel refuses to remove a cgroup while processes remain in it, and the
        // worker is killed asynchronously. So the directory is removed on a helper
        // thread, which retries until the processes have gone. The drop itself does
        // not block. `cgroup.kill` (Linux 5.14 and later) kills whatever is left.
        let path = std::mem::take(&mut self.path);
        let _ = std::fs::write(path.join("cgroup.kill"), "1");
        std::thread::spawn(move || {
            for _ in 0..CGROUP_REMOVE_ATTEMPTS {
                if !path.exists() || std::fs::remove_dir(&path).is_ok() {
                    return;
                }
                std::thread::sleep(CGROUP_REMOVE_INTERVAL);
            }
        });
    }
}

/// How often, and how many times, removing a worker cgroup is retried. Ten seconds
/// in all, which is long enough for a killed worker to be reaped.
#[cfg(target_os = "linux")]
const CGROUP_REMOVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);
#[cfg(target_os = "linux")]
const CGROUP_REMOVE_ATTEMPTS: usize = 500;

/// The controllers the worker limits need.
#[cfg(target_os = "linux")]
const REQUIRED_CONTROLLERS: [&str; 3] = ["cpu", "memory", "pids"];

/// Environment variable naming the delegated cgroup that worker cgroups are created under.
#[cfg(target_os = "linux")]
pub const CGROUP_ROOT_ENV: &str = "SANDBOX_CGROUP_ROOT";

/// The delegated cgroup that worker cgroups are created under.
///
/// cgroup v2 lets a cgroup's children use a controller only when the controller is
/// enabled in the parent's `cgroup.subtree_control`, and only while the parent holds
/// no processes directly. Doing that needs the parent to be owned by the host, which
/// is what a systemd unit with `Delegate=yes` gets. The host reads that cgroup from
/// [`CGROUP_ROOT_ENV`] and checks it. It does not move processes around to make one.
#[cfg(target_os = "linux")]
fn delegated_root() -> Result<std::path::PathBuf, SandboxError> {
    let root = std::env::var_os(CGROUP_ROOT_ENV).ok_or_else(|| {
        SandboxError::new(
            ErrorCode::Unsupported,
            format!(
                "worker cgroups need {CGROUP_ROOT_ENV}, a delegated cgroup v2 directory with cpu, memory, and pids enabled in its cgroup.subtree_control. A systemd unit with Delegate=yes provides one"
            ),
        )
    })?;
    let root = std::path::PathBuf::from(root);
    let enabled = read_controllers(&root.join("cgroup.subtree_control"))?;
    for controller in REQUIRED_CONTROLLERS {
        if !enabled.iter().any(|c| c == controller) {
            return Err(SandboxError::new(
                ErrorCode::Unsupported,
                format!(
                    "{controller} is not enabled in {}/cgroup.subtree_control",
                    root.display()
                ),
            ));
        }
    }
    Ok(root)
}

#[cfg(target_os = "linux")]
fn read_controllers(file: &std::path::Path) -> Result<Vec<String>, SandboxError> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| SandboxError::internal(format!("reading {}: {e}", file.display())))?;
    Ok(text.split_whitespace().map(str::to_string).collect())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Runs `check` in a forked child and returns its exit code. The child exits
    /// with `_exit`, so no test-harness code runs in it. `check` must not return
    /// into the harness.
    fn in_child(check: impl FnOnce() -> i32) -> i32 {
        // SAFETY: the child runs only `check`, which installs a filter and makes
        // syscalls, and then calls `_exit`. It never returns into the harness.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let code = check();
            unsafe { libc::_exit(code) };
        }
        let mut status = 0;
        // SAFETY: `status` is a valid out-pointer for the wait.
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        assert_eq!(waited, pid);
        assert!(libc::WIFEXITED(status), "the child did not exit normally");
        libc::WEXITSTATUS(status)
    }

    #[test]
    fn the_seccomp_filter_denies_ptrace_and_unshare_with_eperm() {
        let code = in_child(|| {
            if install_seccomp().is_err() {
                return 10;
            }
            // SAFETY: PTRACE_TRACEME takes no pointers. The filter should deny it.
            let rc = unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
            let ptrace_errno = std::io::Error::last_os_error().raw_os_error();
            if rc != -1 || ptrace_errno != Some(libc::EPERM) {
                return 11;
            }
            // SAFETY: unshare takes only flags. The filter should deny it too.
            let rc = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
            let unshare_errno = std::io::Error::last_os_error().raw_os_error();
            if rc != -1 || unshare_errno != Some(libc::EPERM) {
                return 12;
            }
            0
        });
        assert_eq!(code, 0, "denied syscalls did not fail with EPERM");
    }

    #[test]
    fn the_seccomp_filter_still_allows_ordinary_calls() {
        let code = in_child(|| {
            if install_seccomp().is_err() {
                return 10;
            }
            // SAFETY: getpid takes no arguments and cannot fail.
            if unsafe { libc::getpid() } <= 0 {
                return 11;
            }
            // Memory and file calls a script needs must keep working.
            let file = std::fs::File::open("/proc/self/status");
            if file.is_err() {
                return 12;
            }
            0
        });
        assert_eq!(code, 0);
    }
}
