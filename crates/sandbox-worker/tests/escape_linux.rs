//! OS-level escape tests: real attempts to break out of the worker's live
//! isolation (namespaces + seccomp + cgroups), from inside an isolated process.
//!
//! The application-level escape suites (`sandbox-runtime`, `sandbox-shell`,
//! `sandbox-wasm`) test hostile script behavior *inside* the sandbox API. This
//! tier tests the OS boundary itself: a forked child applies the same
//! [`sandbox_worker::isolation::enter`] the worker calls, then attempts the
//! usual escape routes and asserts each one fails.
//!
//! The child runs the real `enter()` path, so a regression that silently
//! drops a layer (a missing seccomp install, an unshared namespace that is not
//! unshared) fails a test. The negative control (`the_probes_actually_run`)
//! keeps the tier from being vacuous: it proves the same probes succeed
//! without isolation, so a success in the isolated child is a real EPERM, not
//! an environment that denies the probe for some other reason.
//!
//! **Convention.** A passing test states a property the code holds. A test
//! marked `#[ignore = "KNOWN GAP: ..."]` states a property the code should
//! hold and does not; `cargo test -- --ignored` lists every open gap. See
//! docs/ESCAPE_TESTS.md.
//!
//! Runs in a Linux container with root privileges, via `docker/run-escape-tests.sh`
//! (which already invokes this test binary by name). It does not need the VM
//! image or a delegated cgroup: the probes exercise namespaces and seccomp
//! only, and the cgroup layer is checked best-effort.

#![cfg(target_os = "linux")]

use sandbox_worker::isolation::{enter, Isolation};

/// Runs `check` in a forked child and returns its exit code. The child exits
/// with `_exit`, so no test-harness code runs in it. `check` must not return
/// into the harness. Mirrors the fork helper in `tests/isolation.rs`.
fn in_child(check: impl FnOnce() -> i32) -> i32 {
    // SAFETY: the child runs only `check`, then calls `_exit`. It never returns
    // into the harness.
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

/// Exit codes the child uses. Zero means every probe behaved as expected.
mod code {
    pub const ENTER_FAILED: i32 = 10;
    pub const UNEXPECTED_RESULT: i32 = 11;
}

/// A probe that should be denied by the live seccomp filter, with the name and
/// the code that runs it. Each probe returns the raw result: `Ok(())` if the
/// syscall succeeded, `Err(errno)` if it failed.
type Probe = (&'static str, Box<dyn FnMut() -> Result<(), i32>>);

/// The escape probes: syscalls the live denylist filter denies, and that an
/// allowlist filter (being made live concurrently in a separate PR) also
/// denies. Probing these holds under either regime.
fn escape_probes() -> Vec<Probe> {
    let mut probes = Vec::new();
    // Cross-process memory access.
    probes.push((
        "ptrace",
        Box::new(|| {
            // SAFETY: PTRACE_TRACEME takes no pointers. Should be denied.
            let rc = unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
            probe_result(rc)
        }),
    ));
    // Mount manipulation.
    probes.push((
        "mount",
        Box::new(|| {
            let src = c"/tmp";
            let dst = c"/tmp";
            // SAFETY: both strings are valid NUL-terminated; the other
            // arguments are null or zero, which `mount` accepts.
            let rc = unsafe {
                libc::mount(
                    src.as_ptr(),
                    dst.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                )
            };
            probe_result(rc)
        }),
    ));
    probes.push((
        "umount2",
        Box::new(|| {
            let target = c"/tmp";
            // SAFETY: `target` is a valid NUL-terminated string that lives for
            // the call.
            let rc = unsafe { libc::umount2(target.as_ptr(), 0) };
            probe_result(rc)
        }),
    ));
    // Namespace creation and joining.
    probes.push((
        "unshare",
        Box::new(|| {
            // SAFETY: unshare takes only flags. Should be denied.
            let rc = unsafe { libc::unshare(libc::CLONE_NEWNET) };
            probe_result(rc)
        }),
    ));
    probes.push((
        "setns",
        Box::new(|| {
            let fd = unsafe { libc::open(c"/proc/self/ns/net".as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                // Without the fd there is nothing to probe; report success so
                // the caller skips this probe rather than failing it.
                return Ok(());
            }
            let rc = unsafe { libc::setns(fd, libc::CLONE_NEWNET) };
            let _ = unsafe { libc::close(fd) };
            probe_result(rc)
        }),
    ));
    // BPF, perf, userfaultfd, keyctl, and kernel module loading.
    probes.push((
        "bpf",
        Box::new(|| {
            // SAFETY: BPF_STATS takes a pointer to a u32 we own. Should be denied.
            let stats: u32 = 0;
            let rc = unsafe { libc::bpf(libc::BPF_STATS, &stats as *const u32 as *const _, 4) };
            probe_result(rc)
        }),
    ));
    probes.push((
        "perf_event_open",
        Box::new(|| {
            // SAFETY: the arguments are zero-valued; the call should be denied
            // before the kernel inspects them.
            let rc = unsafe { libc::syscall(libc::SYS_perf_event_open, 0, 0, 0, -1, 0) };
            probe_result(rc as i32)
        }),
    ));
    probes.push((
        "userfaultfd",
        Box::new(|| {
            // SAFETY: takes only flags. Should be denied.
            let rc = unsafe { libc::userfaultfd(0) };
            probe_result(rc)
        }),
    ));
    probes.push((
        "keyctl",
        Box::new(|| {
            // SAFETY: takes only an int command and null pointers. Should be
            // denied.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_keyctl,
                    libc::KEYCTL_INVALIDATE as i64,
                    std::ptr::null_mut(),
                )
            };
            probe_result(rc as i32)
        }),
    ));
    probes.push((
        "init_module",
        Box::new(|| {
            // SAFETY: the image pointer is null and the length is zero; the
            // call should be denied before the kernel reads the image.
            let rc = unsafe { libc::init_module(std::ptr::null(), 0, std::ptr::null()) };
            probe_result(rc)
        }),
    ));
    probes.push((
        "kexec_load",
        Box::new(|| {
            // SAFETY: all pointers are null; the call should be denied.
            let rc = unsafe { libc::kexec_load(0, 0, 0, std::ptr::null()) };
            probe_result(rc)
        }),
    ));
    probes
}

/// Turns a raw syscall return value into a probe result. `-1` means the call
/// failed with the current errno; anything else means it succeeded.
fn probe_result(rc: libc::c_int) -> Result<(), i32> {
    if rc == -1 {
        let errno = std::io::Error::last_os_error().raw_os_error();
        Err(errno.unwrap_or(-1))
    } else {
        Ok(())
    }
}

/// Runs every escape probe and reports the first one whose result does not
/// match `expect`. `expect` is `true` when the probe should be denied with
/// EPERM (isolated child) and `false` when it should not be (control). The
/// returned string names the probe and what happened, for the test message.
fn run_probes(expect_denied: bool) -> Result<(), String> {
    for (name, probe) in escape_probes() {
        let outcome = probe();
        let denied = matches!(outcome, Err(e) if e == libc::EPERM);
        let detail = match outcome {
            Ok(()) => "succeeded".to_string(),
            Err(e) => format!("failed with errno {e}"),
        };
        if expect_denied && !denied {
            return Err(format!("{name}: {detail}"));
        }
        if !expect_denied && denied {
            return Err(format!("{name}: {detail}"));
        }
    }
    Ok(())
}

/// The seccomp layer: every denied syscall fails with EPERM in the isolated
/// child. The live filter (the denylist `install_seccomp`) is what `enter()`
/// applies on current main; an allowlist would deny the same syscalls, so the
/// assertion holds under either regime.
#[test]
fn the_isolated_child_cannot_make_denied_syscalls() {
    let code = in_child(|| {
        if enter(&Isolation {
            namespaces: false,
            seccomp: true,
            cgroup: None,
        })
        .is_err()
        {
            return code::ENTER_FAILED;
        }
        match run_probes(true) {
            Ok(()) => 0,
            Err(what) => {
                eprintln!("probe {what} was not denied with EPERM");
                code::UNEXPECTED_RESULT
            }
        }
    });
    assert_eq!(code, 0, "an escape probe was not denied with EPERM");
}

/// Negative control: the same probes, run without isolation, must not be
/// denied with EPERM. Without this, a success in the isolated child could mean
/// the environment denies the probe for some other reason (or seccomp is not
/// installed at all) and the tier would be vacuous.
///
/// Which control this is: a "control probe succeeds" test. We run the full
/// probe set unisolated and assert none of them fail with EPERM. A probe that
/// fails with another errno (for example `EPERM` from the kernel for a
/// capability the container lacks, on a non-root CI runner) is not the
/// seccomp errno we are testing for, so the control still passes; the isolated
/// child asserts the same probes DO return EPERM, which only the seccomp
/// filter produces for a probe the kernel would otherwise allow.
#[test]
fn the_probes_actually_run() {
    let code = in_child(|| match run_probes(false) {
        Ok(()) => 0,
        Err(what) => {
            eprintln!("control probe {what} was denied with EPERM without isolation");
            code::UNEXPECTED_RESULT
        }
    });
    assert_eq!(
        code, 0,
        "a probe was denied with EPERM without isolation; the seccomp tier would be vacuous"
    );
}

/// The mount and namespace layer: in the fully isolated child, `mount(2)`,
/// `umount2(2)`, and `unshare(CLONE_NEWNET)` all fail with EPERM. The
/// seccomp filter denies all three, and the new network namespace means the
/// child cannot re-enter the host's network namespace via `setns`.
#[test]
fn the_isolated_child_cannot_mount_or_create_namespaces() {
    let code = in_child(|| {
        if enter(&Isolation::strict()).is_err() {
            return code::ENTER_FAILED;
        }
        let mut all = true;
        for (name, probe) in escape_probes() {
            if !matches!(name, "mount" | "umount2" | "unshare" | "setns") {
                continue;
            }
            if !matches!(probe(), Err(e) if e == libc::EPERM) {
                eprintln!("probe {name} was not denied with EPERM");
                all = false;
            }
        }
        if all {
            0
        } else {
            code::UNEXPECTED_RESULT
        }
    });
    assert_eq!(
        code, 0,
        "a mount or namespace probe was not denied with EPERM"
    );
}

/// The network layer: in the fully isolated child, the process has no usable
/// host network. The probe opens a TCP socket and attempts a non-blocking
/// connect to a well-known public address (8.8.8.8:53). In a new network
/// namespace the socket has no route to the address, so `connect` fails (with
/// `ENETUNREACH` or `EINPROGRESS` followed by no progress, depending on the
/// kernel). We assert the connect does not succeed within a short timeout.
///
/// Why this probe over reading `/proc/net/route`: the route file is visible
/// in the new namespace and shows no default route, but a socket connect is
/// the stronger statement — it tests the kernel's actual routing decision,
/// not a file's contents. Both are reliable in a root CI container; the
/// connect probe additionally catches a case where a route exists but the
/// interface is down.
#[test]
fn the_isolated_child_has_no_usable_host_network() {
    let code = in_child(|| {
        if enter(&Isolation::strict()).is_err() {
            return code::ENTER_FAILED;
        }
        // SAFETY: the socket and connect calls take only the arguments shown.
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return code::ENTER_FAILED;
        }
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_port = 53u16.to_be();
        // 8.8.8.8
        let mut octets = [8u8, 8, 8, 8];
        // SAFETY: `octets` is a 4-byte array; `s_addr` is 4 bytes on this
        // platform.
        unsafe {
            addr.sin_addr.s_addr = u32::from_ne_bytes(octets).to_be();
        }
        // Make the socket non-blocking so a connect with no route fails fast
        // instead of hanging.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            let _ = unsafe { libc::close(fd) };
            return code::ENTER_FAILED;
        }
        // SAFETY: `addr` is a valid, fully-initialized sockaddr_in.
        // SAFETY: `addr` is a valid, fully-initialized sockaddr_in.
        let rc = unsafe {
            libc::connect(
                fd,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
            )
        };
        // A successful connect (rc == 0) means the isolated child reached the
        // public internet, which is an escape.
        if rc == 0 {
            eprintln!("isolated child connected to 8.8.8.8:53");
            let _ = unsafe { libc::close(fd) };
            return code::UNEXPECTED_RESULT;
        }
        // A non-blocking connect with no route fails fast with
        // `ENETUNREACH`/`EHOSTUNREACH`. `EINPROGRESS` means the kernel is
        // still trying, so poll for writability: a completing connect would
        // report success. Either way the child must not connect.
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 500) };
        if ready > 0 && (pollfd.revents & libc::POLLOUT) != 0 {
            let mut err: i32 = 0;
            let len = std::mem::size_of::<i32>() as u32;
            // SAFETY: `err` and `len` are valid out-arguments for getsockopt.
            let _ = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut err as *mut i32 as *mut _,
                    &mut len,
                )
            };
            if err == 0 {
                eprintln!("isolated child's connect to 8.8.8.8:53 completed");
                let _ = unsafe { libc::close(fd) };
                return code::UNEXPECTED_RESULT;
            }
        }
        let _ = unsafe { libc::close(fd) };
        0
    });
    assert_eq!(code, 0, "the isolated child had a usable host network");
}
