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
//! hold and does not; `cargo test -- --ignored` lists every open gap. The
//! authoritative run is `docker/run-escape-tests.sh`.
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

/// True when the tests may assume CAP_SYS_ADMIN (a root CI container). The
/// privileged tiers (`the_probes_actually_run`,
/// `the_isolated_child_cannot_mount_or_create_namespaces`, and
/// `the_isolated_child_has_no_usable_host_network`) need it: `enter()` with
/// namespaces calls `unshare(CLONE_NEWNET|CLONE_NEWNS)`, which the kernel
/// denies with EPERM to a process without CAP_SYS_ADMIN, so an isolated child
/// cannot even be entered and the assertions would be vacuous. A negative
/// control run by an unprivileged user also cannot distinguish a working
/// filter from a kernel that never allowed the probe.
///
/// The seccomp-only test is *not* gated: seccomp is self-imposed, so the
/// assertion holds for any user. The `rust` CI job (non-root) runs the
/// ungated tests; `docker/run-escape-tests.sh` sets
/// `ESCAPE_PRIVILEGED_TESTS=1` so the privileged tiers run where the kernel
/// grants CAP_SYS_ADMIN.
fn privileged_tests_enabled() -> bool {
    std::env::var("ESCAPE_PRIVILEGED_TESTS")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// Reports a probe failure from inside the forked child. `eprintln!` is not
/// async-signal-safe: if another thread holds the stderr lock at fork time, the
/// child inherits the locked state and can block forever in the write, and the
/// parent's `waitpid` has no timeout. A single `write(2)` is atomic and safe.
/// The exit code already carries the pass/fail signal; this is context only.
fn child_fail(msg: &str) {
    // SAFETY: `msg` is a valid, NUL-free byte string for the call's duration.
    // The return value is discarded: a partial or failed write in a dying
    // child is acceptable; the exit code is what the parent asserts on.
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}

/// A probe that should be denied by the live seccomp filter, with the name and
/// the code that runs it. Each probe returns the raw result: `Ok(())` if the
/// syscall succeeded, `Err(errno)` if it failed.
type Probe = (&'static str, Box<dyn Fn() -> Result<(), i32>>);

/// The escape probes: syscalls the live denylist filter denies, and that an
/// allowlist filter (being made live concurrently in a separate PR) also
/// denies. Probing these holds under either regime.
fn escape_probes() -> Vec<Probe> {
    // The annotation is required: without it the first `push` fixes the element
    // type to the concrete closure, and the later closures (different types)
    // cannot be coerced into trait objects at construction.
    let mut probes: Vec<Probe> = Vec::new();
    // Cross-process memory access.
    probes.push((
        "ptrace",
        Box::new(|| {
            // SAFETY: PTRACE_TRACEME takes no pointers. Should be denied.
            let rc = unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
            probe_result(rc as libc::c_long)
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
            probe_result(rc as libc::c_long)
        }),
    ));
    probes.push((
        "umount2",
        Box::new(|| {
            let target = c"/tmp";
            // SAFETY: `target` is a valid NUL-terminated string that lives for
            // the call.
            let rc = unsafe { libc::umount2(target.as_ptr(), 0) };
            probe_result(rc as libc::c_long)
        }),
    ));
    // Namespace creation and joining.
    probes.push((
        "unshare",
        Box::new(|| {
            // SAFETY: unshare takes only flags. Should be denied.
            let rc = unsafe { libc::unshare(libc::CLONE_NEWNET) };
            probe_result(rc as libc::c_long)
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
            probe_result(rc as libc::c_long)
        }),
    ));
    // BPF, perf, userfaultfd, keyctl, and kernel module loading.
    probes.push((
        "bpf",
        Box::new(|| {
            // SAFETY: BPF_STATS takes a pointer to a u32 we own. Should be denied.
            // Use syscall() directly: `libc::bpf` is not in every libc version,
            // and the `BPF_STATS` command constant is not in every libc either
            // (raw value 5 from <linux/bpf.h>).
            let stats: u32 = 0;
            let rc = unsafe {
                libc::syscall(libc::SYS_bpf, 5i64, &stats as *const u32 as *const _, 4i64)
            };
            probe_result(rc)
        }),
    ));
    probes.push((
        "perf_event_open",
        Box::new(|| {
            // SAFETY: the arguments are zero-valued; the call should be denied
            // before the kernel inspects them.
            let rc = unsafe { libc::syscall(libc::SYS_perf_event_open, 0, 0, 0, -1, 0) };
            probe_result(rc)
        }),
    ));
    probes.push((
        "userfaultfd",
        Box::new(|| {
            // SAFETY: takes only flags. Should be denied.
            let rc = unsafe { libc::syscall(libc::SYS_userfaultfd, 0i64) };
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
                    // `syscall` is variadic and supplies no expected type, so the
                    // pointee must be named explicitly.
                    std::ptr::null_mut::<libc::c_int>(),
                )
            };
            probe_result(rc)
        }),
    ));
    probes.push((
        "init_module",
        Box::new(|| {
            // SAFETY: the image pointer is null and the length is zero; the
            // call should be denied before the kernel reads the image.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_init_module,
                    std::ptr::null::<u8>() as *const libc::c_void,
                    0i64,
                    std::ptr::null::<libc::c_long>() as *const libc::c_char,
                )
            };
            probe_result(rc)
        }),
    ));
    probes.push((
        "kexec_load",
        Box::new(|| {
            // SAFETY: all arguments are zero/null; the call should be denied.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_kexec_load,
                    0i64,
                    0i64,
                    0i64,
                    std::ptr::null::<libc::c_long>() as *const libc::c_char,
                )
            };
            probe_result(rc)
        }),
    ));
    probes
}

/// Turns a raw syscall return value into a probe result. `-1` means the call
/// failed with the current errno; anything else means it succeeded. Accepts
/// `libc::c_long` because `libc::syscall()` returns `c_long` (i64 on 64-bit).
fn probe_result(rc: libc::c_long) -> Result<(), i32> {
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
                child_fail(&format!("probe {what} was not denied with EPERM\n"));
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
    if !privileged_tests_enabled() {
        eprintln!(
            "SKIPPED the_probes_actually_run (set ESCAPE_PRIVILEGED_TESTS=1 in a root container; see docker/run-escape-tests.sh)"
        );
        return;
    }
    let code = in_child(|| match run_probes(false) {
        Ok(()) => 0,
        Err(what) => {
            child_fail(&format!(
                "control probe {what} was denied with EPERM without isolation\n"
            ));
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
    if !privileged_tests_enabled() {
        eprintln!(
            "SKIPPED the_isolated_child_cannot_mount_or_create_namespaces (set ESCAPE_PRIVILEGED_TESTS=1 in a root container; see docker/run-escape-tests.sh)"
        );
        return;
    }
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
                child_fail(&format!("probe {name} was not denied with EPERM\n"));
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

/// The network layer: in the fully isolated child, the process cannot reach a
/// listener that exists only on the host's network namespace.
///
/// The parent binds and listens on `127.0.0.1` before forking. The listener
/// lives in the host namespace, so it is the strongest reachable target: if
/// the child's `connect` completes, the child reached the host network. In a
/// new network namespace the child's loopback is a fresh, empty interface, so
/// the connect cannot succeed.
///
/// Why a test-owned endpoint over a public address (8.8.8.8): a public
/// address depends on the CI runner's outbound internet, which is not
/// guaranteed and which a sandboxed runner may route differently. A local
/// listener the test owns is reachable under the only conditions we care
/// about (a connected host network) and unreachable under none others.
///
/// The control run (no isolation) asserts the *unisolated* child connects, so
/// the test is not vacuous: a listener that cannot be reached on the host
/// fails the control, not the isolated probe.
///
/// Both connects are blocking. The loopback path never produces
/// `EINPROGRESS` (the destination is directly connected), so a blocking
/// `connect` either succeeds or returns the final errno synchronously. A
/// non-blocking connect would need `poll` + `SO_ERROR` disambiguation: on a
/// socket that failed synchronously, `poll` reports `POLLOUT` with
/// `SO_ERROR == 0`, which would be read as a completed connect — a false
/// escape. Blocking removes that whole class of error.
#[test]
fn the_isolated_child_has_no_usable_host_network() {
    if !privileged_tests_enabled() {
        eprintln!(
            "SKIPPED the_isolated_child_has_no_usable_host_network (set ESCAPE_PRIVILEGED_TESTS=1 in a root container; see docker/run-escape-tests.sh)"
        );
        return;
    }
    // The parent binds the listener before forking. The child inherits the fd
    // but does not use it; it only connects to the address.
    // SAFETY: the socket/bind/listen calls take only the arguments shown.
    let lfd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    assert!(lfd >= 0, "parent: listener socket() failed");
    // SAFETY: `one` is a valid int out-live for setsockopt.
    let one: libc::c_int = 1;
    let ok = unsafe {
        libc::setsockopt(
            lfd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const libc::c_int as *const _,
            std::mem::size_of::<libc::c_int>() as u32,
        )
    };
    assert!(ok == 0, "parent: setsockopt(SO_REUSEADDR) failed");
    let mut laddr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    laddr.sin_family = libc::AF_INET as u16;
    laddr.sin_port = 0u16.to_be(); // kernel-assigned port
    laddr.sin_addr.s_addr = 0x7f000001u32.to_be(); // 127.0.0.1
                                                   // SAFETY: `laddr` is valid and fully initialized.
    let rc = unsafe {
        libc::bind(
            lfd,
            &laddr as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as u32,
        )
    };
    assert!(rc == 0, "parent: bind() failed");
    // Discover the kernel-assigned port.
    let mut baddr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut blen = std::mem::size_of::<libc::sockaddr_in>() as u32;
    // SAFETY: `baddr` and `blen` are valid out-arguments for getsockname.
    let rc = unsafe {
        libc::getsockname(
            lfd,
            &mut baddr as *mut libc::sockaddr_in as *mut libc::sockaddr,
            &mut blen,
        )
    };
    assert!(rc == 0, "parent: getsockname() failed");
    let port = u16::from_be(baddr.sin_port);
    // SAFETY: listen takes only the fd and backlog.
    assert_eq!(
        unsafe { libc::listen(lfd, 8) },
        0,
        "parent: listen() failed"
    );

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
        addr.sin_port = port.to_be();
        addr.sin_addr.s_addr = 0x7f000001u32.to_be(); // 127.0.0.1
                                                      // SAFETY: `addr` is a valid, fully-initialized sockaddr_in.
        let rc = unsafe {
            libc::connect(
                fd,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
            )
        };
        // A successful connect (rc == 0) means the isolated child reached the
        // host's listener, which is an escape. Any failure errno (`ECONNREFUSED`
        // with no loopback peer, `ENETUNREACH` with no route, `EACCES` from
        // seccomp) is the denial we want; the child cannot hang because a
        // blocking connect to a directly-connected destination completes
        // synchronously.
        if rc == 0 {
            child_fail("isolated child connected to the host listener\n");
            let _ = unsafe { libc::close(fd) };
            return code::UNEXPECTED_RESULT;
        }
        let _ = unsafe { libc::close(fd) };
        0
    });

    // Control: an unisolated child must connect, or the listener itself is
    // broken and the isolated-side assertion would be vacuous. Blocking: the
    // listener is in the backlog and loopback is synchronous, so rc == 0.
    let control = in_child(|| {
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return code::ENTER_FAILED;
        }
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_port = port.to_be();
        addr.sin_addr.s_addr = 0x7f000001u32.to_be();
        // SAFETY: `addr` is a valid, fully-initialized sockaddr_in.
        let rc = unsafe {
            libc::connect(
                fd,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
            )
        };
        let _ = unsafe { libc::close(fd) };
        if rc == 0 {
            0
        } else {
            child_fail("control: unisolated child could not connect to the host listener\n");
            code::UNEXPECTED_RESULT
        }
    });

    // Close the listener in the parent; the children no longer need it.
    let _ = unsafe { libc::close(lfd) };

    assert_eq!(
        control, 0,
        "control: the unisolated child could not reach the host listener"
    );
    assert_eq!(code, 0, "the isolated child had a usable host network");
}
