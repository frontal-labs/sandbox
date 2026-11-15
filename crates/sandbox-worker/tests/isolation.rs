//! Integration tests for the Linux seccomp filter. The filter is installed in a
//! forked child, because it applies only to the installing thread and a failing
//! syscall would not recover in the harness.

#[cfg(target_os = "linux")]
use sandbox_worker::isolation::install_seccomp;

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
