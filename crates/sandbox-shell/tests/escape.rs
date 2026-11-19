//! Escape tests for the shell and VFS (SECURITY_MODEL.md, "Required before any public launch").
//!
//! A hostile script runs in a real `Shell` over a real `Vfs`, and the test checks, from outside,
//! that it could not touch the host: no host file read or written, no host environment value,
//! no host process signalled, no network connection made. Then it checks the resource limits
//! (steps, output, string size, inodes, file size, nesting) hold against hostile input.
//!
//! The probes use *canaries on the real host*: a file with known contents, an environment
//! variable with a known value, a sacrificial child process, and a TCP listener on loopback. A
//! probe passes only if the canary is untouched, so the test cannot pass because it asked the
//! wrong question.
//!
//! **Convention.** A passing test states a property the code holds. A test marked
//! `#[ignore = "KNOWN GAP: ..."]` states the property the code should hold and does not;
//! `cargo test -- --ignored` lists every open gap. See docs/ESCAPE_TESTS.md.

use sandbox_cas::MemStore;
use sandbox_shell::interpreter::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, Quotas, SessionFs, Vfs};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CANARY: &str = "HOST-CANARY-b61d09e5";

fn new_vfs() -> Arc<Vfs> {
    Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())))
}

fn shell_over(vfs: Arc<Vfs>) -> Shell {
    Shell::new(vfs, ShellConfig::default())
}

struct Ran {
    code: i32,
    out: String,
    err: String,
}

impl Ran {
    fn all(&self) -> String {
        format!("{}{}", self.out, self.err)
    }
}

async fn run_on(shell: &mut Shell, script: &str) -> Ran {
    let result = tokio::time::timeout(Duration::from_secs(180), shell.run(script))
        .await
        .unwrap_or_else(|_| panic!("script timed out after 180s: {script}"));
    // A script the shell refuses to parse or expand is a refusal, not a failure of the test.
    let result = match result {
        Ok(result) => result,
        Err(e) => {
            return Ran {
                code: 2,
                out: String::new(),
                err: format!("refused: {}", e.message),
            }
        }
    };
    Ran {
        code: result.exit_code,
        out: String::from_utf8_lossy(&result.stdout).into_owned(),
        err: String::from_utf8_lossy(&result.stderr).into_owned(),
    }
}

async fn run(script: &str) -> Ran {
    run_on(&mut shell_over(new_vfs()), script).await
}

async fn put(vfs: &Vfs, path: &str, data: &[u8]) {
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let handle = vfs.open(Path::new(path), flags).await.unwrap();
    vfs.write(&handle, 0, data).await.unwrap();
    vfs.close(&handle);
}

/// A directory on the real host holding a canary file.
struct HostCanary {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
}

impl HostCanary {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("escape-shell-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("canary.txt");
        std::fs::write(&file, CANARY).unwrap();
        Self { dir, file }
    }

    fn path(&self) -> &str {
        self.file.to_str().unwrap()
    }

    /// The host directory has exactly the canary, unchanged.
    fn assert_untouched(&self) {
        let names: Vec<_> = std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["canary.txt".to_string()],
            "host directory changed"
        );
        assert_eq!(std::fs::read_to_string(&self.file).unwrap(), CANARY);
    }
}

impl Drop for HostCanary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A TCP listener on host loopback. A hostile script that reaches the network connects to it.
struct Listener {
    inner: std::net::TcpListener,
}

impl Listener {
    fn new() -> Self {
        let inner = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        inner.set_nonblocking(true).unwrap();
        Self { inner }
    }

    fn port(&self) -> u16 {
        self.inner.local_addr().unwrap().port()
    }

    fn assert_no_connection(&self) {
        // The OS completes a TCP handshake into the backlog even before accept, so a connection
        // attempt is visible here as soon as it was made.
        match self.inner.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok((_, from)) => panic!("the sandbox opened a TCP connection to the host from {from}"),
            Err(e) => panic!("listener error: {e}"),
        }
    }
}

// ==========================================================================================
// Host filesystem
// ==========================================================================================

#[tokio::test]
async fn no_path_form_reaches_a_host_file() {
    let canary = HostCanary::new("read");
    let p = canary.path().to_string();
    let dir = canary.dir.to_str().unwrap().to_string();
    let climbing = format!("{}{}", "../".repeat(40), p.trim_start_matches('/'));
    let scripts = vec![
        format!("cat {p}"),
        format!("cat < {p}"),
        format!("cat {climbing}"),
        format!("cd {dir} && cat canary.txt"),
        format!("cd / && cat ./{}", p.trim_start_matches('/')),
        format!(". {p}"),
        format!("source {p}"),
        format!("sh {p}"),
        format!("echo $(cat {p})"),
        format!("echo $(< {p})"),
        format!("head -n1 {p}"),
        format!("tail -n1 {p}"),
        format!("grep -r CANARY {dir}"),
        "grep -r HOST-CANARY /".to_string(),
        "find / -name canary.txt".to_string(),
        format!("find {dir}"),
        "ls -R /".to_string(),
        format!("ls {dir}"),
        format!("cp {p} /copy && cat /copy"),
        format!("sed -n p {p}"),
        format!("base64 {p}"),
        format!("wc -c {p}"),
        format!("stat {p}"),
        format!("realpath {p}"),
        format!("diff {p} {p}"),
        format!("sort {p}"),
        format!("echo {p} | xargs cat"),
        format!("read x < {p}; echo $x"),
        format!("while read l; do echo $l; done < {p}"),
        format!("wasm {p}"),
        format!("node {p}"),
    ];
    for script in scripts {
        let mut shell = shell_over(new_vfs());
        let ran = run_on(&mut shell, &script).await;
        assert!(
            !ran.all().contains(CANARY),
            "the host canary leaked through `{script}`: {}",
            ran.all()
        );
    }
    canary.assert_untouched();
}

#[tokio::test]
async fn the_control_script_can_read_a_file_that_is_in_the_vfs() {
    // Control for the test above: the same commands work on a VFS file, so the refusals are real.
    let vfs = new_vfs();
    put(&vfs, "/data.txt", CANARY.as_bytes()).await;
    let mut shell = shell_over(vfs);
    for script in [
        "cat /data.txt",
        "head -n1 /data.txt",
        "grep HOST-CANARY /data.txt",
        "sort /data.txt",
    ] {
        let ran = run_on(&mut shell, script).await;
        assert!(
            ran.out.contains(CANARY),
            "`{script}` did not read the VFS file: {}",
            ran.all()
        );
    }
}

#[tokio::test]
async fn no_host_file_can_be_written_deleted_or_replaced() {
    let canary = HostCanary::new("write");
    let p = canary.path().to_string();
    let d = canary.dir.to_str().unwrap().to_string();
    let scripts = vec![
        format!("echo pwn > {d}/pwn.txt"),
        format!("echo pwn >> {p}"),
        format!("echo pwn > {p}"),
        format!("touch {d}/touched"),
        format!("mkdir -p {d}/sub/dir"),
        format!("rm {p}"),
        format!("rm -rf {d}"),
        format!("mv {p} /moved"),
        format!("echo x > /a; mv /a {p}"),
        format!("echo x > /a; cp /a {d}/copied"),
        format!("echo x | tee {d}/teed"),
        format!("echo x > /a; sed -i s/x/y/ {p}"),
        format!("printf pwn | base64 > {d}/b64"),
        format!("echo x > /a; cp /a {p}"),
    ];
    for script in scripts {
        let mut shell = shell_over(new_vfs());
        let _ = run_on(&mut shell, &script).await;
        canary.assert_untouched();
    }
}

#[tokio::test]
async fn the_vfs_root_starts_empty_and_names_no_host_directory() {
    let mut shell = shell_over(new_vfs());
    let ran = run_on(&mut shell, "ls /").await;
    assert_eq!(ran.out.trim(), "", "a fresh root has entries: {}", ran.out);
    for host_dir in [
        "/Users", "/home", "/private", "/var", "/proc", "/sys", "/root", "/etc", "/usr", "/bin",
        "/Volumes",
    ] {
        let ran = run_on(&mut shell, &format!("test -e {host_dir}")).await;
        assert_ne!(ran.code, 0, "{host_dir} exists in a fresh VFS");
    }
}

#[tokio::test]
async fn proc_and_dev_pseudo_files_are_not_the_hosts() {
    for script in [
        "cat /proc/self/environ",
        "cat /proc/self/maps",
        "cat /proc/self/cmdline",
        "cat /proc/version",
        "cat /proc/cpuinfo",
        "cat /proc/self/cgroup",
        "ls /proc/self/fd",
        "cat /sys/class/net/eth0/address",
        "cat /etc/hostname",
        "cat /etc/shadow",
        "cat /etc/passwd",
        "cat /etc/hosts",
    ] {
        let ran = run(script).await;
        assert_ne!(ran.code, 0, "`{script}` succeeded: {}", ran.out);
        assert!(ran.out.trim().is_empty(), "`{script}` printed {}", ran.out);
    }
}

#[tokio::test]
async fn dev_tcp_redirects_do_not_open_sockets() {
    // bash opens a socket for a redirect to /dev/tcp/HOST/PORT. This shell must not.
    let listener = Listener::new();
    let port = listener.port();
    for script in [
        format!("echo hi > /dev/tcp/127.0.0.1/{port}"),
        format!("cat < /dev/tcp/127.0.0.1/{port}"),
        format!("exec 3<>/dev/tcp/127.0.0.1/{port}; echo hi >&3"),
        format!("echo hi > /dev/udp/127.0.0.1/{port}"),
        format!("cat /dev/tcp/localhost/{port}"),
    ] {
        let _ = run(&script).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    listener.assert_no_connection();
}

#[tokio::test]
async fn network_clients_do_not_exist_and_nothing_connects() {
    let listener = Listener::new();
    let port = listener.port();
    let mut shell = shell_over(new_vfs());
    for tool in [
        "curl", "wget", "nc", "ncat", "netcat", "telnet", "ssh", "scp", "ftp", "ping", "dig",
        "nslookup", "host", "python3", "python", "perl", "ruby", "php", "bash", "zsh", "busybox",
        "git", "openssl", "socat", "docker", "kubectl",
    ] {
        let script = format!("{tool} http://127.0.0.1:{port}/ 127.0.0.1 {port}");
        let ran = run_on(&mut shell, &script).await;
        assert!(
            ran.code == 127 || ran.code == 126,
            "`{tool}` ran (exit {}): {}",
            ran.code,
            ran.all()
        );
    }
    // `node` is a builtin. Without a contained runner it refuses; with one it has no network op.
    let ran = run_on(
        &mut shell,
        &format!("node -e \"require('net').connect({port}, '127.0.0.1')\""),
    )
    .await;
    assert_ne!(ran.code, 0);
    tokio::time::sleep(Duration::from_millis(200)).await;
    listener.assert_no_connection();
}

#[tokio::test]
async fn host_environment_is_invisible() {
    std::env::set_var("FRONTAL_ESCAPE_CANARY", CANARY);
    std::env::set_var("AWS_SECRET_ACCESS_KEY", CANARY);
    let mut shell = shell_over(new_vfs());
    for script in [
        "env",
        "printenv",
        "printenv FRONTAL_ESCAPE_CANARY",
        "printenv AWS_SECRET_ACCESS_KEY",
        "export -p",
        "set",
        "echo $FRONTAL_ESCAPE_CANARY",
        "echo ${FRONTAL_ESCAPE_CANARY}",
        "echo ${FRONTAL_ESCAPE_CANARY:-unset}",
        "echo \"${!FRONTAL*}\"",
        "declare -p",
        "cat /proc/self/environ",
        "sh -c 'echo $FRONTAL_ESCAPE_CANARY'",
        "echo $(env)",
        "node -e \"console.log(JSON.stringify(process.env))\"",
        "echo \"[$HOME][$USER][$SHELL][$PATH][$PWD][$HOSTNAME][$LOGNAME]\"",
    ] {
        let ran = run_on(&mut shell, script).await;
        assert!(
            !ran.all().contains(CANARY),
            "a host environment value leaked through `{script}`: {}",
            ran.all()
        );
        assert!(
            !ran.all().contains("/Users/") && !ran.all().contains("/home/"),
            "a host path leaked through `{script}`: {}",
            ran.all()
        );
    }
    // With no session environment configured, `env` prints nothing.
    let ran = run_on(&mut shell, "env").await;
    assert_eq!(ran.out.trim(), "", "env: {}", ran.out);
}

#[tokio::test]
async fn host_process_ids_are_not_exposed() {
    let host = std::process::id().to_string();
    let mut shell = shell_over(new_vfs());
    for script in [
        "echo $$",
        "echo $PPID",
        "echo $BASHPID",
        "echo $UID $EUID",
        "echo $RANDOM $SECONDS",
    ] {
        let ran = run_on(&mut shell, script).await;
        assert!(
            !ran.out.split_whitespace().any(|w| w == host),
            "`{script}` printed the host pid {host}: {}",
            ran.out
        );
    }
}

#[tokio::test]
async fn no_command_can_signal_a_host_process() {
    // A sacrificial host process. A script that can signal processes would end it.
    let mut victim = tokio::task::spawn_blocking(|| {
        std::process::Command::new("sleep")
            .arg("600")
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();
    let pid = victim.id();
    let mut shell = shell_over(new_vfs());
    for script in [
        format!("kill -9 {pid}"),
        format!("kill {pid}"),
        format!("kill -KILL -- {pid}"),
        format!("exec kill -9 {pid}"),
        "pkill sleep".to_string(),
        "killall sleep".to_string(),
        format!("trap 'kill -9 {pid}' EXIT; true"),
        format!("wait {pid}"),
        format!("echo {pid} | xargs kill -9"),
        format!("sh -c 'kill -9 {pid}'"),
    ] {
        let _ = run_on(&mut shell, &script).await;
        assert!(
            victim.try_wait().unwrap().is_none(),
            "the sacrificial process was killed by `{script}`"
        );
    }
    victim.kill().unwrap();
    let _ = victim.wait();
}

#[tokio::test]
async fn a_file_with_a_shebang_does_not_run_on_the_host() {
    let canary = HostCanary::new("shebang");
    let vfs = new_vfs();
    let d = canary.dir.to_str().unwrap();
    put(
        &vfs,
        "/evil.sh",
        format!("#!/bin/sh\ntouch {d}/from-script\n").as_bytes(),
    )
    .await;
    put(
        &vfs,
        "/evil",
        format!("#!/usr/bin/env sh\ntouch {d}/from-env\n").as_bytes(),
    )
    .await;
    let mut shell = shell_over(vfs);
    for script in [
        "/evil.sh",
        "./evil.sh",
        "sh /evil.sh",
        "/evil",
        "exec /evil.sh",
        "command /evil.sh",
    ] {
        let _ = run_on(&mut shell, script).await;
    }
    canary.assert_untouched();
}

#[tokio::test]
async fn symlinks_resolve_inside_the_vfs_and_cycles_terminate() {
    let canary = HostCanary::new("links");
    let vfs = new_vfs();
    let host = canary.path().to_string();
    vfs.symlink(Path::new(&host), Path::new("/abs"))
        .await
        .unwrap();
    let climbing = format!("{}{}", "../".repeat(40), host.trim_start_matches('/'));
    vfs.symlink(Path::new(&climbing), Path::new("/climb"))
        .await
        .unwrap();
    vfs.symlink(Path::new("/b"), Path::new("/a")).await.unwrap();
    vfs.symlink(Path::new("/a"), Path::new("/b")).await.unwrap();
    vfs.mkdir(Path::new("/d"), 0o755).await.unwrap();
    put(&vfs, "/d/file", b"x").await;
    vfs.symlink(Path::new("/d"), Path::new("/d/self"))
        .await
        .unwrap();
    vfs.symlink(Path::new("/"), Path::new("/d/root"))
        .await
        .unwrap();
    // A chain of 500 links to a real file.
    put(&vfs, "/end", b"end").await;
    let mut previous = "/end".to_string();
    for i in 0..500 {
        let link = format!("/chain{i}");
        vfs.symlink(Path::new(&previous), Path::new(&link))
            .await
            .unwrap();
        previous = link;
    }
    let mut shell = shell_over(vfs);
    for script in [
        "cat /abs",
        "cat /climb",
        "cat /a",
        "cat /b",
        "ls /a",
        "cat /d/self/self/self/file",
        "cat /chain499",
        "ls -R /d",
        "find /d",
        "du -a /d",
        "grep -r x /d",
        "cp -r /d /copy",
        "rm -r /d",
        "find /",
    ] {
        let started = Instant::now();
        let ran = run_on(&mut shell, script).await;
        assert!(
            !ran.all().contains(CANARY),
            "`{script}` leaked the host file: {}",
            ran.all()
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "`{script}` took {:?}",
            started.elapsed()
        );
    }
    canary.assert_untouched();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_symlink_race_never_reaches_the_host() {
    // One task flips /r between a directory holding a file and a symlink to a host path. The
    // others read through it. Whichever interleaving happens, no host content may come back.
    let canary = HostCanary::new("race");
    let vfs = new_vfs();
    vfs.mkdir(Path::new("/r"), 0o755).await.unwrap();
    put(&vfs, "/r/f", b"inside").await;
    let host_dir = canary.dir.clone();
    let flipper = {
        let vfs = vfs.clone();
        tokio::spawn(async move {
            for i in 0..400 {
                if i % 2 == 0 {
                    let _ = vfs.rm(Path::new("/r"), true).await;
                    let _ = vfs.symlink(&host_dir, Path::new("/r")).await;
                } else {
                    let _ = vfs.rm(Path::new("/r"), true).await;
                    let _ = vfs.mkdir(Path::new("/r"), 0o755).await;
                    put(&vfs, "/r/f", b"inside").await;
                }
            }
        })
    };
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let vfs = vfs.clone();
            tokio::spawn(async move {
                let mut shell = shell_over(vfs);
                let mut leaked = String::new();
                for _ in 0..150 {
                    let result = shell.run("cat /r/f; cat /r/canary.txt; ls /r").await;
                    if let Ok(r) = result {
                        let text = String::from_utf8_lossy(&r.stdout).into_owned();
                        if text.contains(CANARY) {
                            leaked = text;
                        }
                    }
                }
                leaked
            })
        })
        .collect();
    flipper.await.expect("the flipper panicked");
    for reader in readers {
        let leaked = reader.await.expect("a reader panicked");
        assert!(
            leaked.is_empty(),
            "host data came back through the race: {leaked}"
        );
    }
}

// ==========================================================================================
// Resource exhaustion
// ==========================================================================================

#[tokio::test]
async fn endless_loops_and_recursion_end() {
    for script in [
        "while true; do :; done",
        "until false; do :; done",
        "for i in $(seq 1 100000000); do :; done",
        "f() { f; }; f",
        "f() { f | f; }; f",
        "f() { f; f; }; f",
        "f() { g; }; g() { f; }; f",
        ":(){ :|:; };:",
    ] {
        let config = ShellConfig {
            max_steps: 50_000,
            ..ShellConfig::default()
        };
        let mut shell = Shell::new(new_vfs(), config);
        let ran = run_on(&mut shell, script).await;
        assert_ne!(ran.code, 0, "`{script}` ended with success");
    }
}

#[tokio::test]
async fn the_step_limit_stops_a_busy_loop() {
    // About 7 microseconds of CPU per step on a debug build, so the default 1,000,000 steps is
    // seconds of CPU. Wall time is longer on a loaded host, because every step yields.
    let config = ShellConfig {
        max_steps: 20_000,
        ..ShellConfig::default()
    };
    let mut shell = Shell::new(new_vfs(), config);
    let ran = run_on(&mut shell, "while true; do :; done").await;
    assert_ne!(ran.code, 0);
    assert!(ran.err.contains("step limit"), "{}", ran.err);
}

#[tokio::test]
async fn output_floods_stop_at_the_cap() {
    let config = ShellConfig {
        max_output_bytes: 256 * 1024,
        ..ShellConfig::default()
    };
    let mut shell = Shell::new(new_vfs(), config);
    for script in [
        "while true; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done",
        "while true; do echo xxxxxxxxxxxxxxxx >&2; done",
        "f() { echo 0123456789; f; f; }; f",
    ] {
        let ran = run_on(&mut shell, script).await;
        assert!(
            ran.out.len() + ran.err.len() <= 256 * 1024 + 4096,
            "`{script}` produced {} bytes",
            ran.out.len() + ran.err.len()
        );
        assert_ne!(ran.code, 0);
    }
}

#[tokio::test]
#[ignore = "KNOWN GAP: variable size is not bounded; doubling a string 27 times builds 128 MiB (40 doublings would be 1 TiB)"]
async fn a_string_cannot_be_doubled_without_bound() {
    // 27 doublings is 128 MiB: large enough to show there is no cap, small enough to run safely.
    let ran = run("s=x; i=0; while [ $i -lt 27 ]; do s=$s$s; i=$((i+1)); done; echo ${#s}").await;
    let length: u64 = ran.out.trim().parse().unwrap_or(0);
    eprintln!("string length reached: {length} bytes; stderr: {}", ran.err);
    assert!(
        length < 64 * 1024 * 1024,
        "a variable grew to {length} bytes with no refusal"
    );
}

#[tokio::test]
async fn a_command_substitution_cannot_hold_unbounded_output() {
    // ~170 MB of output held in a variable. The shell's output budget is 8 MiB.
    let ran = run("x=$(seq 1 20000000); echo ${#x}").await;
    let length: u64 = ran.out.trim().parse().unwrap_or(0);
    eprintln!("substitution held {length} bytes; stderr: {}", ran.err);
    assert!(
        length < 16 * 1024 * 1024,
        "substitution held {length} bytes"
    );
}

#[tokio::test]
async fn printf_width_cannot_allocate_without_bound() {
    let ran = run("printf '%200000000d' 1 | wc -c").await;
    let length: u64 = ran.out.trim().parse().unwrap_or(0);
    eprintln!("printf produced {length} bytes; stderr: {}", ran.err);
    assert!(length < 16 * 1024 * 1024, "printf produced {length} bytes");
}

#[tokio::test]
async fn arithmetic_never_panics_the_host() {
    // Overflow and shift edge cases. A panic inside `Shell::run` would take down the task that
    // serves the session.
    let exprs = [
        "9223372036854775807 + 1",
        "-9223372036854775808 - 1",
        "9223372036854775807 * 2",
        "-9223372036854775808 / -1",
        "-9223372036854775808 % -1",
        "1 / 0",
        "1 % 0",
        "1 << 64",
        "1 << 63",
        "1 << -1",
        "1 >> 64",
        "1 >> -1",
        "2 ** 63",
        "2 ** 64",
        "2 ** -1",
        "0 ** -1",
        "-9223372036854775808 ** 2",
        "- -9223372036854775808",
        "9223372036854775807 + 9223372036854775807",
        "99999999999999999999",
        "-99999999999999999999",
        "0x7fffffffffffffff + 1",
        "0xffffffffffffffffff",
        "1 ? 2",
        "((((((((((1))))))))))",
        "~-9223372036854775808",
        "!9223372036854775807",
    ];
    for expr in exprs {
        let script = format!("echo $(( {expr} ))");
        let handle = tokio::spawn(async move {
            let mut shell = shell_over(new_vfs());
            shell.run(&script).await.map(|r| r.exit_code)
        });
        let joined = handle.await;
        assert!(
            joined.is_ok(),
            "arithmetic `{expr}` panicked the shell task: {:?}",
            joined.err()
        );
    }
}

#[tokio::test]
async fn vfs_quotas_bound_inodes_file_size_total_bytes_and_depth() {
    let quotas = Quotas {
        total_bytes: 256 * 1024,
        inode_count: 100,
        max_file_size: 64 * 1024,
        max_path_depth: 16,
    };
    let vfs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())).with_quotas(quotas));
    let mut shell = shell_over(vfs.clone());

    // Inode exhaustion: 500 files against a limit of 100.
    let _ = run_on(&mut shell, "for i in $(seq 1 500); do touch /f$i; done").await;
    let ran = run_on(&mut shell, "ls / | wc -l").await;
    let files: u64 = ran.out.trim().parse().unwrap();
    assert!(
        files <= 100,
        "{files} files were created against an inode limit of 100"
    );

    // File size: 100,000 bytes against a 64 KiB per-file limit.
    let ran = run_on(
        &mut shell,
        "rm -rf /f*; printf '%100000s' x > /big; echo $?",
    )
    .await;
    let size = vfs
        .stat(Path::new("/big"))
        .await
        .map(|s| s.size)
        .unwrap_or(0);
    assert!(
        size <= 64 * 1024,
        "a file of {size} bytes was written ({})",
        ran.all()
    );

    // Total bytes: ten 60 KiB files against a 256 KiB total.
    let _ = run_on(&mut shell, "rm -f /big").await;
    let _ = run_on(
        &mut shell,
        "for i in 1 2 3 4 5 6 7 8 9 10; do printf '%61440s' x > /t$i; done",
    )
    .await;
    let mut total = 0u64;
    for i in 1..=10 {
        total += vfs
            .stat(Path::new(&format!("/t{i}")))
            .await
            .map(|s| s.size)
            .unwrap_or(0);
    }
    assert!(
        total <= 256 * 1024,
        "{total} bytes stored against a 256 KiB total"
    );

    // Path depth: 300 levels against a limit of 16.
    let deep = "d/".repeat(300);
    let _ = run_on(&mut shell, &format!("mkdir -p /{deep}")).await;
    let ran = run_on(&mut shell, &format!("test -d /{deep}")).await;
    assert_ne!(
        ran.code, 0,
        "a path 300 levels deep was created against a limit of 16"
    );
}

#[tokio::test]
async fn truncating_a_file_to_a_huge_size_is_refused() {
    // A sparse-file request must hit the file-size quota, not allocate.
    let vfs = new_vfs();
    put(&vfs, "/f", b"x").await;
    let result = vfs.truncate(Path::new("/f"), 1 << 40).await;
    assert!(result.is_err(), "truncate to 1 TiB succeeded");
    let result = vfs.truncate(Path::new("/f"), u64::MAX).await;
    assert!(result.is_err(), "truncate to u64::MAX succeeded");
}

// --- Tool manifest tampering -----------------------------------------------------------

/// Counts to 500 million, then exits 0: about 1.5e9 instructions, 15 times the default fuel.
const COUNT_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (func (export "_start")
    (local $i i32)
    (loop $l
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 500000000))))
    (call $exit (i32.const 0))))
"#;

/// Grows memory to 512 MiB (8192 pages) and exits 0 if it could.
const GROW_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory 1)
  (func (export "_start")
    (if (i32.eq (memory.grow (i32.const 8192)) (i32.const -1))
      (then (call $exit (i32.const 1))))
    (call $exit (i32.const 0))))
"#;

#[tokio::test]
async fn the_default_wasm_limits_stop_a_guest_that_runs_too_long_or_grows_too_big() {
    // Control for the manifest test below.
    let vfs = new_vfs();
    put(&vfs, "/count.wasm", COUNT_WAT.as_bytes()).await;
    put(&vfs, "/grow.wasm", GROW_WAT.as_bytes()).await;
    let mut shell = shell_over(vfs);
    let ran = run_on(&mut shell, "wasm /count.wasm").await;
    assert_eq!(
        ran.code,
        1,
        "default fuel did not stop the counter: {}",
        ran.all()
    );
    let ran = run_on(&mut shell, "wasm /grow.wasm").await;
    assert_eq!(
        ran.code,
        1,
        "default memory cap did not stop the grower: {}",
        ran.all()
    );
}

#[tokio::test]
#[ignore = "KNOWN GAP: /etc/tools.toml is read from the guest-writable VFS, so a guest sets its own fuel, memory and output limits"]
async fn a_guest_cannot_raise_its_own_limits_by_writing_the_tool_manifest() {
    let vfs = new_vfs();
    put(&vfs, "/count.wasm", COUNT_WAT.as_bytes()).await;
    put(&vfs, "/grow.wasm", GROW_WAT.as_bytes()).await;
    vfs.mkdir(Path::new("/etc"), 0o755).await.unwrap();
    let mut shell = shell_over(vfs);
    // The guest writes the manifest itself, with no hash pin and generous limits.
    let ran = run_on(
        &mut shell,
        "printf '[[tool]]\\nname = \"count\"\\npath = \"/count.wasm\"\\nfuel = 100000000000\\n\\n[[tool]]\\nname = \"grow\"\\npath = \"/grow.wasm\"\\nmax_memory_bytes = 1073741824\\n' > /etc/tools.toml",
    )
    .await;
    assert_eq!(ran.code, 0, "writing the manifest failed: {}", ran.all());
    let count = run_on(&mut shell, "count").await;
    let grow = run_on(&mut shell, "grow").await;
    eprintln!(
        "count exit {} ({}), grow exit {} ({})",
        count.code,
        count.err.trim(),
        grow.code,
        grow.err.trim()
    );
    assert_eq!(
        count.code, 1,
        "the guest ran 15x past the default fuel by editing its manifest"
    );
    assert_eq!(
        grow.code, 1,
        "the guest grew to 512 MiB past the default cap by editing its manifest"
    );
}

// --- Parser and expansion bombs, in a child process ----------------------------------------------

/// How a child probe ended.
#[derive(Debug)]
struct ChildEnd {
    success: bool,
    signal: Option<i32>,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

fn child_probe(name: &str) -> ChildEnd {
    use std::os::unix::process::ExitStatusExt;
    let started = Instant::now();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--nocapture", "--test-threads=1"])
        .env("ESCAPE_SHELL_CHILD", name)
        .output()
        .expect("the child probe starts");
    ChildEnd {
        success: out.status.success(),
        signal: out.status.signal(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}

/// A script that nests one construct `depth` deep.
fn nested_script(kind: &str, depth: usize) -> String {
    let n = depth;
    match kind {
        "subshell" => format!("{}true{}", "(".repeat(n), ")".repeat(n)),
        "brace_group" => format!("{}true;{}", "{ ".repeat(n), " }".repeat(n)),
        "if" => format!("{}true;{}", "if true; then ".repeat(n), " fi".repeat(n)),
        "while" => format!("{}true;{}", "while true; do ".repeat(n), " done".repeat(n)),
        "for" => format!("{}true;{}", "for i in 1; do ".repeat(n), " done".repeat(n)),
        "case" => format!("{}true;{}", "case x in x) ".repeat(n), " ;; esac".repeat(n)),
        "arith_parens" => format!("echo $(( {}1{} ))", "(".repeat(n), ")".repeat(n)),
        "arith_subst" => format!("echo {}1{}", "$((".repeat(n), "))".repeat(n)),
        "param_default" => format!("echo {}x{}", "${a:-".repeat(n), "}".repeat(n)),
        "backticks" => format!("echo {}x{}", "`echo ".repeat(n), "`".repeat(n)),
        "double_quotes_subst" => format!("echo {}x{}", "\"$(echo ".repeat(n), ")\"".repeat(n)),
        "pipeline" => format!("true{}", " | cat".repeat(n)),
        "and_list" => format!("true{}", " && true".repeat(n)),
        "function_chain" => {
            (0..n)
                .map(|i| format!("f{i}() {{ f{}; }}; ", i + 1))
                .collect::<String>()
                + "f0"
        }
        other => panic!("unknown kind {other}"),
    }
}

/// The entry point of the child probes. It does nothing in a normal run.
#[test]
fn child_entry() {
    let Ok(name) = std::env::var("ESCAPE_SHELL_CHILD") else {
        return;
    };
    // The same stack a tokio worker has: the default 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        let handle = tokio::spawn(async move {
            let script = if let Some(rest) = name.strip_prefix("nest:") {
                let (kind, depth) = rest.split_once(':').unwrap();
                nested_script(kind, depth.parse().unwrap())
            } else {
                panic!("unknown child probe {name}")
            };
            let mut shell = shell_over(new_vfs());
            match tokio::time::timeout(Duration::from_secs(60), shell.run(&script)).await {
                Ok(Ok(r)) => println!("CHILD-RESULT exit={}", r.exit_code),
                Ok(Err(e)) => println!("CHILD-RESULT error={}", e.message),
                Err(_) => println!("CHILD-RESULT timeout"),
            }
        });
        if let Err(e) = handle.await {
            println!("CHILD-RESULT task-panicked={e}");
        }
    });
}

fn nesting_failures(kinds: &[&str]) -> Vec<String> {
    let mut failures = Vec::new();
    for kind in kinds {
        for depth in [100usize, 20_000, 200_000] {
            let end = child_probe(&format!("nest:{kind}:{depth}"));
            if !end.success
                || !end.stdout.contains("CHILD-RESULT")
                || end.stdout.contains("task-panicked")
            {
                failures.push(format!(
                    "{kind} depth {depth}: signal {:?} (6 is SIGABRT, a stack overflow), after {:?}",
                    end.signal, end.elapsed
                ));
            }
            let _ = end.stderr;
        }
    }
    failures
}

#[test]
fn deeply_nested_constructs_do_not_overflow_the_stack() {
    let failures = nesting_failures(&[
        "subshell",
        "brace_group",
        "if",
        "while",
        "for",
        "case",
        "backticks",
        "double_quotes_subst",
        "pipeline",
        "and_list",
        "function_chain",
    ]);
    assert!(
        failures.is_empty(),
        "the host process died or a task panicked:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "KNOWN GAP: arithmetic and ${x:-...} expansion recurse without a depth limit; 1,000 levels abort the process with a stack overflow"]
fn deeply_nested_arithmetic_and_parameter_expansion_do_not_overflow_the_stack() {
    let failures = nesting_failures(&["arith_parens", "arith_subst", "param_default"]);
    assert!(
        failures.is_empty(),
        "the host process aborted:\n{}",
        failures.join("\n")
    );
}
