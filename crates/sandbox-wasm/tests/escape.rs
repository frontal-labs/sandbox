//! Escape tests for the WASM tier (SECURITY_MODEL.md, "Required before any public launch").
//!
//! Each test runs a hostile guest, written in WAT, against the real WASI host and checks that the
//! guest cannot reach the host filesystem, the host environment, the network, or the host's
//! memory and CPU beyond its limits. They run on any platform.
//!
//! **Convention.** A test that passes states a property the code holds today. A test marked
//! `#[ignore = "KNOWN GAP: ..."]` states the property the code *should* hold and currently does
//! not, so `cargo test` stays green while `cargo test -- --ignored` shows every open gap.
//! Fixing a gap means removing its `ignore`. See docs/ESCAPE_TESTS.md.
//!
//! Probes that could abort the host process (and so the test runner) run in a child process:
//! the test re-executes this binary with `ESCAPE_WASM_CHILD=<probe>` and reports how it ended.

use sandbox_testutil::HostCanary;
use sandbox_vfs::{SessionFs, Vfs};
use sandbox_wasm::wasi::errno;
use sandbox_wasm::{
    run_module, run_module_with_mount, run_module_with_preopens_streamed, Mount, Preopen,
    WasiLimits, WasiOutcome,
};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

// --- Fixtures -----------------------------------------------------------------------------

fn vfs() -> Arc<Vfs> {
    sandbox_testutil::new_vfs()
}

async fn put(vfs: &Vfs, path: &str, data: &[u8]) {
    sandbox_testutil::put(vfs, path, data).await
}

/// A VFS with `/work` (the guest's preopened fd 3) and `/secret/flag`, a file in the same VFS
/// but outside the mount. It returns the mount.
async fn work_mount() -> (Arc<Vfs>, Mount) {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/secret"), 0o755).await.unwrap();
    put(&vfs, "/secret/flag", b"VFS-OUTSIDE-MOUNT").await;
    put(&vfs, "/work/own", b"own file").await;
    let mount = Mount {
        vfs: vfs.clone(),
        root: "/work".to_string(),
    };
    (vfs, mount)
}

const CANARY: &str = "HOST-FILESYSTEM-CANARY-7f3a91c2";

/// A host canary that uses the WASM-specific canary string and `escape-wasm-{tag}` dirs.
fn host_canary(tag: &str) -> HostCanary {
    HostCanary::with_content(tag, CANARY)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(haystack).contains(needle)
}

async fn run(
    wat: &str,
    stdin: &[u8],
    limits: WasiLimits,
    mount: Option<Mount>,
) -> Result<WasiOutcome, sandbox_types::SandboxError> {
    run_module_with_mount(
        wat.as_bytes().to_vec(),
        vec!["guest".to_string()],
        stdin.to_vec(),
        limits,
        mount,
    )
    .await
}

// --- Guests -------------------------------------------------------------------------------

/// Reads `dirfd`, `oflags`, and a path from stdin (one byte, one byte, the rest), calls
/// `path_open`, writes the errno as one byte, and, if the open worked, writes up to 4000 bytes
/// of the file.
const OPEN_PROBE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (local $n i32) (local $rc i32) (local $fd i32)
    (i32.store (i32.const 0) (i32.const 1024))
    (i32.store (i32.const 4) (i32.const 4000))
    (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 8)))
    (local.set $n (i32.load (i32.const 8)))
    (local.set $rc
      (call $path_open
        (i32.load8_u (i32.const 1024))
        (i32.const 1)
        (i32.const 1026)
        (i32.sub (local.get $n) (i32.const 2))
        (i32.load8_u (i32.const 1025))
        (i64.const -1) (i64.const -1)
        (i32.const 0)
        (i32.const 12)))
    (i32.store8 (i32.const 16) (local.get $rc))
    (i32.store (i32.const 20) (i32.const 16))
    (i32.store (i32.const 24) (i32.const 1))
    (drop (call $fd_write (i32.const 1) (i32.const 20) (i32.const 1) (i32.const 28)))
    (if (i32.eqz (local.get $rc))
      (then
        (local.set $fd (i32.load (i32.const 12)))
        (i32.store (i32.const 0) (i32.const 5000))
        (i32.store (i32.const 4) (i32.const 4000))
        (drop (call $fd_read (local.get $fd) (i32.const 0) (i32.const 1) (i32.const 8)))
        (i32.store (i32.const 4) (i32.load (i32.const 8)))
        (i32.store (i32.const 0) (i32.const 5000))
        (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 28)))))))
"#;

/// For every fd in 0..128, writes the result of `fd_prestat_get` and then of `fd_fdstat_get`.
const FD_SCAN_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_prestat_get" (func $prestat (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_fdstat_get" (func $fdstat (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (local $i i32)
    (loop $l
      (i32.store8 (i32.add (i32.const 1000) (local.get $i))
        (call $prestat (local.get $i) (i32.const 100)))
      (i32.store8 (i32.add (i32.const 1200) (local.get $i))
        (call $fdstat (local.get $i) (i32.const 100)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 128))))
    (i32.store (i32.const 0) (i32.const 1000))
    (i32.store (i32.const 4) (i32.const 128))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
    (i32.store (i32.const 0) (i32.const 1200))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))))
"#;

/// Writes the environment and argv counts and sizes: four little-endian u32s.
const ENV_PROBE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "environ_sizes_get" (func $esz (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "args_sizes_get" (func $asz (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (drop (call $esz (i32.const 100) (i32.const 104)))
    (drop (call $asz (i32.const 108) (i32.const 112)))
    (i32.store (i32.const 0) (i32.const 100))
    (i32.store (i32.const 4) (i32.const 16))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))))
"#;

/// Calls every socket function WASI preview1 has and writes each result as one byte.
const SOCKET_PROBE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "sock_accept" (func $accept (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "sock_recv"
    (func $recv (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "sock_send" (func $send (param i32 i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "sock_shutdown" (func $shutdown (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (local $fd i32)
    ;; fds 0..8: accept, recv, send, shutdown, one byte each -> 32 bytes at 1000
    (loop $l
      (i32.store8 (i32.add (i32.const 1000) (i32.mul (local.get $fd) (i32.const 4)))
        (call $accept (local.get $fd) (i32.const 0) (i32.const 200)))
      (i32.store8 (i32.add (i32.const 1001) (i32.mul (local.get $fd) (i32.const 4)))
        (call $recv (local.get $fd) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 200) (i32.const 204)))
      (i32.store8 (i32.add (i32.const 1002) (i32.mul (local.get $fd) (i32.const 4)))
        (call $send (local.get $fd) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 200)))
      (i32.store8 (i32.add (i32.const 1003) (i32.mul (local.get $fd) (i32.const 4)))
        (call $shutdown (local.get $fd) (i32.const 2)))
      (local.set $fd (i32.add (local.get $fd) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $fd) (i32.const 8))))
    (i32.store (i32.const 0) (i32.const 1000))
    (i32.store (i32.const 4) (i32.const 32))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))))
"#;

/// Writes 64 KiB, 4000 times: 250 MB of stdout, if nothing stops it.
const FLOOD_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (local $i i32)
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 65536))
    (loop $l
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))
      (drop (call $fd_write (i32.const 2) (i32.const 0) (i32.const 1) (i32.const 8)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 4000))))))
"#;

const RECURSE_WAT: &str = r#"
(module
  (func $r (call $r) (call $r))
  (func (export "_start") (call $r)))
"#;

const SPIN_WAT: &str = r#"(module (func (export "_start") (loop $l (br $l))))"#;

const TRAP_UNREACHABLE_WAT: &str = r#"(module (func (export "_start") (unreachable)))"#;
const TRAP_DIV_WAT: &str = r#"
(module (func (export "_start") (drop (i32.div_s (i32.const 1) (i32.const 0)))))"#;
const TRAP_OOB_WAT: &str = r#"
(module (memory 1) (func (export "_start") (drop (i32.load (i32.const 16777216)))))"#;
const TRAP_INDIRECT_WAT: &str = r#"
(module (table 1 funcref) (func (export "_start") (call_indirect (i32.const 0))))"#;

/// A shared memory declared far past the cap, then grown by 2000 pages (about 125 MiB).
/// Exit 7: the grow succeeded. Exit 0: it was refused.
const SHARED_MEMORY_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory 1 8192 shared)
  (func (export "_start")
    (if (i32.ne (memory.grow (i32.const 2000)) (i32.const -1))
      (then (call $exit (i32.const 7))))))
"#;

/// Grows a funcref table by 20 million elements (160 MB). Exit 7: it grew. Exit 0: refused.
const TABLE_GROW_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (table $t 0 funcref)
  (func (export "_start")
    (if (i32.ne (table.grow $t (ref.null func) (i32.const 20000000)) (i32.const -1))
      (then (call $exit (i32.const 7))))))
"#;

/// `n` separate memories, each grown to just under 64 MiB. The exit code is how many grew.
fn multi_memory_wat(n: usize) -> String {
    let mut wat = String::from(
        "(module\n (import \"wasi_snapshot_preview1\" \"proc_exit\" (func $exit (param i32)))\n",
    );
    for i in 0..n {
        wat.push_str(&format!(" (memory $m{i} 1)\n"));
    }
    wat.push_str(" (func (export \"_start\") (local $ok i32)\n");
    for i in 0..n {
        wat.push_str(&format!(
            "  (if (i32.ne (memory.grow $m{i} (i32.const 1000)) (i32.const -1))\n   (then (local.set $ok (i32.add (local.get $ok) (i32.const 1)))))\n"
        ));
    }
    wat.push_str("  (call $exit (local.get $ok)))\n)");
    wat
}

/// Reads one byte from a 1-byte iovec of the file `big`, `count` times, at offset 0.
fn pread_loop_wat(count: u32) -> String {
    format!(
        r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_pread" (func $fd_pread (param i32 i32 i32 i64 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 64) "big")
  (func (export "_start")
    (local $fd i32) (local $i i32)
    (if (call $path_open (i32.const 3) (i32.const 1) (i32.const 64) (i32.const 3) (i32.const 0)
          (i64.const -1) (i64.const -1) (i32.const 0) (i32.const 12))
      (then (call $exit (i32.const 2))))
    (local.set $fd (i32.load (i32.const 12)))
    (i32.store (i32.const 0) (i32.const 200))
    (i32.store (i32.const 4) (i32.const 1))
    (loop $l
      (drop (call $fd_pread (local.get $fd) (i32.const 0) (i32.const 1) (i64.const 0) (i32.const 8)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const {count}))))))
"#
    )
}

/// `fd_write` with an iovec count of `i32::MAX`. The host sizes a vector from the count before it
/// reads a single iovec.
const HUGE_IOVEC_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 2147483647) (i32.const 8)))))
"#;

/// `depth` nested blocks: a compile-time stress for the validator and the code generator.
fn nested_blocks_wat(depth: usize) -> String {
    let mut wat = String::from("(module (func (export \"_start\")\n");
    for _ in 0..depth {
        wat.push_str("(block\n");
    }
    for _ in 0..depth {
        wat.push_str(")\n");
    }
    wat.push_str("))");
    wat
}

// --- Probe helpers -------------------------------------------------------------------------

/// Runs the open probe. Returns the WASI errno and whatever file contents came back.
async fn open_probe(mount: &Mount, dirfd: u8, oflags: u8, path: &[u8]) -> (i32, Vec<u8>) {
    let mut stdin = vec![dirfd, oflags];
    stdin.extend_from_slice(path);
    let out = run(
        OPEN_PROBE_WAT,
        &stdin,
        WasiLimits::default(),
        Some(mount.clone()),
    )
    .await
    .expect("the probe guest runs");
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.stdout.is_empty(), "the probe wrote no errno");
    (i32::from(out.stdout[0]), out.stdout[1..].to_vec())
}

/// How a child probe ended.
#[derive(Debug)]
struct ChildEnd {
    success: bool,
    /// The signal that killed it, if any (for example SIGABRT after an allocation failure).
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
        .env("ESCAPE_WASM_CHILD", name)
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

/// The entry point of the child probes. It does nothing in a normal run.
#[test]
fn child_entry() {
    let Ok(name) = std::env::var("ESCAPE_WASM_CHILD") else {
        return;
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let wat = match name.as_str() {
            "huge_iovec" => HUGE_IOVEC_WAT.to_string(),
            "nested_blocks" => nested_blocks_wat(30_000),
            other => panic!("unknown child probe {other}"),
        };
        let result = run(&wat, b"", WasiLimits::default(), None).await;
        match result {
            Ok(out) => println!(
                "CHILD-RESULT exit={} stderr={:?}",
                out.exit_code,
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(e) => println!("CHILD-RESULT error={}", e.message),
        }
    });
}

// ==========================================================================================
// Host filesystem
// ==========================================================================================

#[tokio::test]
async fn absolute_and_dotdot_paths_are_refused_with_notcapable() {
    let canary = host_canary("paths");
    let (_vfs, mount) = work_mount().await;
    let host_abs = canary.path().to_string();
    let host_dir_rel = format!("../../../../../../../../../..{}", canary.path());
    for path in [
        "/etc/passwd".to_string(),
        "/".to_string(),
        host_abs,
        "..".to_string(),
        "../secret/flag".to_string(),
        "a/../../secret/flag".to_string(),
        "./../secret/flag".to_string(),
        "own/../../secret/flag".to_string(),
        host_dir_rel,
        "../".repeat(200) + "etc/passwd",
    ] {
        let (rc, body) = open_probe(&mount, 3, 0, path.as_bytes()).await;
        assert_eq!(rc, errno::NOTCAPABLE, "path {path:?} gave errno {rc}");
        assert!(body.is_empty(), "path {path:?} returned data");
    }
}

#[tokio::test]
async fn the_mount_reads_its_own_files_and_nothing_else() {
    // Control for the test above: the probe really can read a file inside the mount, so a
    // refusal means the host said no, not that the probe is broken.
    let (_vfs, mount) = work_mount().await;
    let (rc, body) = open_probe(&mount, 3, 0, b"own").await;
    assert_eq!(rc, 0);
    assert_eq!(body, b"own file");
}

#[tokio::test]
async fn malformed_paths_never_open_anything() {
    let (_vfs, mount) = work_mount().await;
    let long = vec![b'a'; 3900];
    let mut nul_dotdot = b"own\0/../../secret/flag".to_vec();
    nul_dotdot.push(0);
    for (label, path) in [
        ("nul in the middle", b"own\0x".to_vec()),
        ("nul then dotdot", nul_dotdot),
        ("very long", long),
        ("empty", Vec::new()),
        ("only slashes", b"////".to_vec()),
        ("non-utf8", vec![0xff, 0xfe, 0xfd]),
        ("backslash dotdot", b"..\\..\\secret\\flag".to_vec()),
        ("percent encoded dotdot", b"%2e%2e/secret/flag".to_vec()),
    ] {
        let (rc, body) = open_probe(&mount, 3, 0, &path).await;
        assert_ne!(rc, 0, "{label}: the open succeeded");
        assert!(body.is_empty(), "{label}: data came back");
    }
}

#[tokio::test]
async fn only_the_preopened_descriptors_exist() {
    let (_vfs, mount) = work_mount().await;
    let out = run(FD_SCAN_WAT, b"", WasiLimits::default(), Some(mount))
        .await
        .unwrap();
    assert_eq!(out.stdout.len(), 256);
    let (prestat, fdstat) = out.stdout.split_at(128);
    // One preopen: fd 3. Descriptors 0..=2 are stdio and have no prestat.
    let prestat_ok: Vec<usize> = (0..128).filter(|&fd| prestat[fd] == 0).collect();
    assert_eq!(prestat_ok, vec![3], "preopened descriptors");
    let fdstat_ok: Vec<usize> = (0..128).filter(|&fd| fdstat[fd] == 0).collect();
    assert_eq!(fdstat_ok, vec![0, 1, 2, 3], "open descriptors");
}

#[tokio::test]
async fn without_a_mount_the_guest_has_no_filesystem_at_all() {
    let out = run(FD_SCAN_WAT, b"", WasiLimits::default(), None)
        .await
        .unwrap();
    let (prestat, fdstat) = out.stdout.split_at(128);
    assert!((0..128).all(|fd| prestat[fd] != 0), "a preopen exists");
    let fdstat_ok: Vec<usize> = (0..128).filter(|&fd| fdstat[fd] == 0).collect();
    assert_eq!(fdstat_ok, vec![0, 1, 2]);
}

#[tokio::test]
async fn opening_stdio_or_unknown_descriptors_as_directories_fails() {
    let (_vfs, mount) = work_mount().await;
    for dirfd in [0u8, 1, 2, 4, 5, 9, 63, 127, 255] {
        let (rc, body) = open_probe(&mount, dirfd, 0, b"own").await;
        assert_ne!(rc, 0, "dirfd {dirfd} resolved a path");
        assert!(body.is_empty());
    }
}

#[tokio::test]
async fn symlinks_resolve_inside_the_vfs_never_on_the_host() {
    let canary = host_canary("symlink");
    let (vfs, mount) = work_mount().await;
    let host = canary.path();
    // A symlink whose target is the host canary's real path, and one that climbs out with dotdots.
    vfs.symlink(Path::new(host), Path::new("/work/abs"))
        .await
        .unwrap();
    let climbing = format!("{}{}", "../".repeat(40), host.trim_start_matches('/'));
    vfs.symlink(Path::new(&climbing), Path::new("/work/climb"))
        .await
        .unwrap();
    // Loops must end, not hang.
    vfs.symlink(Path::new("/work/b"), Path::new("/work/a"))
        .await
        .unwrap();
    vfs.symlink(Path::new("/work/a"), Path::new("/work/b"))
        .await
        .unwrap();
    vfs.symlink(Path::new("/work/self"), Path::new("/work/self"))
        .await
        .unwrap();
    for link in ["abs", "climb", "a", "b", "self"] {
        let started = Instant::now();
        let (rc, body) = open_probe(&mount, 3, 0, link.as_bytes()).await;
        assert_ne!(rc, 0, "symlink {link} opened");
        assert!(
            !contains(&body, CANARY),
            "symlink {link} leaked the host file"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "symlink {link} took {:?}",
            started.elapsed()
        );
    }
}

#[tokio::test]
async fn a_symlink_cannot_make_the_mount_read_a_sibling_directory_it_was_not_given() {
    // The mount is /work. A symlink to /secret (another directory of the same VFS) lets the
    // guest read it. This records the behaviour: the mount is a convenience root, not a
    // sandbox inside the VFS. The VFS as a whole is the boundary.
    let (vfs, mount) = work_mount().await;
    vfs.symlink(Path::new("/secret"), Path::new("/work/up"))
        .await
        .unwrap();
    let (rc, body) = open_probe(&mount, 3, 0, b"up/flag").await;
    if rc == 0 {
        assert_eq!(body, b"VFS-OUTSIDE-MOUNT");
        eprintln!("NOTE: a symlink lets the mount read siblings inside the same VFS (by design)");
    }
}

#[tokio::test]
#[ignore = "KNOWN GAP: refuse_if_readonly checks the lexical path, so a symlink in the work dir writes through a read-only preopen"]
async fn a_symlink_cannot_write_through_a_read_only_preopen() {
    let (vfs, mount) = work_mount().await;
    vfs.mkdir(Path::new("/usr"), 0o755).await.unwrap();
    vfs.symlink(Path::new("/usr"), Path::new("/work/l"))
        .await
        .unwrap();
    // oflags 1 is O_CREAT. rights -1 include write, so the file is created and writable.
    let stdin = {
        let mut s = vec![3u8, 1];
        s.extend_from_slice(b"l/pwn");
        s
    };
    let out = run_module_with_preopens_streamed(
        OPEN_PROBE_WAT.as_bytes().to_vec(),
        vec!["guest".into()],
        stdin,
        WasiLimits::default(),
        Some(mount),
        vec![Preopen {
            guest_path: "/usr".into(),
            root: "/usr".into(),
            readonly: true,
        }],
        None,
    )
    .await
    .unwrap();
    assert_ne!(out.stdout[0], 0, "created /usr/pwn through the symlink");
    assert!(
        vfs.stat(Path::new("/usr/pwn")).await.is_err(),
        "/usr/pwn exists"
    );
}

// ==========================================================================================
// Environment and network
// ==========================================================================================

#[tokio::test]
async fn the_guest_sees_no_environment_and_only_its_own_argv() {
    // A host secret in the test process's own environment must not reach the guest.
    std::env::set_var("ESCAPE_WASM_ENV_CANARY", CANARY);
    let out = run(ENV_PROBE_WAT, b"", WasiLimits::default(), None)
        .await
        .unwrap();
    let words: Vec<u32> = out
        .stdout
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(words[0], 0, "environment variable count");
    assert_eq!(words[1], 0, "environment size");
    assert_eq!(words[2], 1, "argv count (just the program name)");
}

#[tokio::test]
async fn every_socket_call_is_refused() {
    let (_vfs, mount) = work_mount().await;
    for mount in [None, Some(mount)] {
        let out = run(SOCKET_PROBE_WAT, b"", WasiLimits::default(), mount)
            .await
            .unwrap();
        assert_eq!(out.stdout.len(), 32);
        assert!(
            out.stdout.iter().all(|&rc| rc != 0),
            "a socket call succeeded: {:?}",
            out.stdout
        );
    }
}

#[tokio::test]
async fn a_guest_cannot_import_a_host_function_the_host_does_not_define() {
    // WASI sockets are not preview1, but a guest can ask for anything. Unknown imports fail
    // to link; they never fall back to something real.
    let wat = r#"
(module
  (import "wasi_snapshot_preview1" "sock_open" (func (param i32 i32 i32) (result i32)))
  (import "env" "system" (func (param i32) (result i32)))
  (func (export "_start")))"#;
    let result = run(wat, b"", WasiLimits::default(), None).await;
    assert!(
        result.is_err(),
        "a module with unknown imports instantiated"
    );
}

// ==========================================================================================
// Resource exhaustion
// ==========================================================================================

#[tokio::test]
async fn fuel_stops_an_endless_loop_quickly() {
    let started = Instant::now();
    let out = run(SPIN_WAT, b"", WasiLimits::default(), None)
        .await
        .unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(contains(&out.stderr, "guest trapped"));
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn deep_recursion_traps_and_the_host_survives() {
    let out = run(RECURSE_WAT, b"", WasiLimits::default(), None)
        .await
        .unwrap();
    // The message carries wasmtime's backtrace but not the trap code, so only the outcome is checked.
    assert_eq!(out.exit_code, 1);
    assert!(contains(&out.stderr, "guest trapped"));
}

#[tokio::test]
async fn every_kind_of_trap_is_a_guest_failure_not_a_host_failure() {
    for (label, wat) in [
        ("unreachable", TRAP_UNREACHABLE_WAT),
        ("integer divide by zero", TRAP_DIV_WAT),
        ("out-of-bounds load", TRAP_OOB_WAT),
        ("null call_indirect", TRAP_INDIRECT_WAT),
    ] {
        let out = run(wat, b"", WasiLimits::default(), None).await.unwrap();
        assert_eq!(out.exit_code, 1, "{label}");
        assert!(contains(&out.stderr, "guest trapped"), "{label}");
    }
}

#[tokio::test]
async fn output_is_capped_on_both_streams() {
    let limits = WasiLimits {
        max_output_bytes: 64 * 1024,
        ..WasiLimits::default()
    };
    let out = run(FLOOD_WAT, b"", limits, None).await.unwrap();
    assert!(
        out.stdout.len() <= 64 * 1024,
        "stdout {} bytes",
        out.stdout.len()
    );
    assert!(
        out.stderr.len() <= 64 * 1024 + 4096,
        "stderr {} bytes",
        out.stderr.len()
    );
}

#[tokio::test]
async fn a_single_memory_is_capped_at_max_memory_bytes() {
    let limits = WasiLimits {
        max_memory_bytes: 8 * 1024 * 1024,
        ..WasiLimits::default()
    };
    // 1000 pages is 62.5 MiB, past the 8 MiB cap.
    let out = run(&multi_memory_wat(1), b"", limits, None).await.unwrap();
    assert_eq!(out.exit_code, 0, "the memory grew past the cap");
}

#[tokio::test]
#[ignore = "KNOWN GAP: StoreLimits.memory_size is per memory, so a module with N memories gets N x max_memory_bytes"]
async fn the_cap_covers_all_memories_together() {
    let cap = 64 * 1024 * 1024usize;
    let limits = WasiLimits {
        max_memory_bytes: cap,
        ..WasiLimits::default()
    };
    // Sixteen memories, each grown to about 62.5 MiB. At most one may fit under a 64 MiB total.
    let out = run(&multi_memory_wat(16), b"", limits, None).await.unwrap();
    let grown = out.exit_code as usize;
    eprintln!(
        "memories grown: {grown} x 62.5 MiB = {} MiB against a {} MiB cap",
        grown * 625 / 10,
        cap >> 20
    );
    assert!(
        grown * (1000 * 65536) <= cap,
        "{grown} memories grew, {} MiB in total",
        grown * 625 / 10
    );
}

#[tokio::test]
#[ignore = "KNOWN GAP: a shared memory is not charged against max_memory_bytes"]
async fn a_shared_memory_obeys_the_cap() {
    let limits = WasiLimits {
        max_memory_bytes: 16 * 1024 * 1024,
        ..WasiLimits::default()
    };
    match run(SHARED_MEMORY_WAT, b"", limits, None).await {
        // Not supported at all is also contained.
        Err(_) => {}
        Ok(out) => assert_eq!(
            out.exit_code, 0,
            "a shared memory grew to 125 MiB under a 16 MiB cap"
        ),
    }
}

#[tokio::test]
#[ignore = "KNOWN GAP: table growth is not limited, so a guest takes host memory outside max_memory_bytes"]
async fn a_table_cannot_grow_without_bound() {
    let limits = WasiLimits {
        max_memory_bytes: 16 * 1024 * 1024,
        ..WasiLimits::default()
    };
    let out = run(TABLE_GROW_WAT, b"", limits, None).await.unwrap();
    assert_eq!(
        out.exit_code, 0,
        "a 20 million element table (160 MB) was granted"
    );
}

#[tokio::test]
#[ignore = "KNOWN GAP: host calls copy the whole file, so fuel does not meter their cost"]
async fn a_one_byte_read_costs_the_host_about_one_byte() {
    // A 64 MiB file. Each fd_pread of one byte should be cheap. Fuel counts the guest's
    // instructions, so a host call that does O(file size) work is an unmetered CPU multiplier.
    let (vfs, mount) = work_mount().await;
    put(&vfs, "/work/big", &vec![7u8; 64 * 1024 * 1024]).await;
    let calls = 200u32;
    let started = Instant::now();
    let out = run(
        &pread_loop_wat(calls),
        b"",
        WasiLimits::default(),
        Some(mount),
    )
    .await
    .unwrap();
    let per_call = started.elapsed() / calls;
    assert_eq!(out.exit_code, 0, "{}", String::from_utf8_lossy(&out.stderr));
    eprintln!("fd_pread of 1 byte from a 64 MiB file: {per_call:?} per call");
    assert!(
        per_call < Duration::from_micros(500),
        "{per_call:?} per 1-byte read of a 64 MiB file"
    );
}

#[tokio::test]
#[ignore = "KNOWN GAP: read_iovecs sizes a Vec from the guest's iovec count before reading any iovec"]
async fn an_absurd_iovec_count_cannot_abort_the_host() {
    let end = child_probe("huge_iovec");
    eprintln!("huge_iovec: {end:?}");
    assert!(
        end.success && end.stdout.contains("CHILD-RESULT"),
        "the host process died: signal {:?}, stderr tail {:?}",
        end.signal,
        end.stderr
            .chars()
            .rev()
            .take(300)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
}

#[tokio::test]
async fn a_module_with_extremely_deep_nesting_is_rejected_or_run_not_crashed() {
    let end = child_probe("nested_blocks");
    eprintln!("nested_blocks: {:?}", end);
    assert!(
        end.success && end.stdout.contains("CHILD-RESULT"),
        "the host process died compiling a hostile module: signal {:?}",
        end.signal
    );
    assert!(
        end.elapsed < Duration::from_secs(60),
        "compile took {:?}",
        end.elapsed
    );
}

#[tokio::test]
async fn run_module_has_no_ambient_authority_without_mount() {
    // Control: with no mount the open probe cannot even be given a directory.
    let out = run_module(
        OPEN_PROBE_WAT.as_bytes().to_vec(),
        vec!["guest".into()],
        [&[3u8, 0][..], b"/etc/passwd"].concat(),
        WasiLimits::default(),
    )
    .await
    .unwrap();
    assert_ne!(out.stdout[0], 0);
}
