//! Integration tests for the WASI preview1 host.

use sandbox_types::ErrorCode;
use sandbox_types::SandboxError;
use sandbox_wasm::wasi::errno;
use sandbox_wasm::wasi::{
    is_cached, resolve_guest_path, run_module, run_module_with_mount_streamed, StreamObserver,
    WasiLimits, WasiOutcome,
};

/// Writes "hi\n" to stdout and exits with status 3.
const HELLO_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "hi\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 3))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))
    (call $proc_exit (i32.const 3))))
"#;

/// Writes "one\n" and then "two\n" to stdout in two separate `fd_write` calls.
const TWO_WRITES_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "one\0a")
  (data (i32.const 16) "two\0a")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 4))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))
    (i32.store (i32.const 0) (i32.const 16))
    (i32.store (i32.const 4) (i32.const 4))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))))
"#;

const SPIN_WAT: &str = r#"
(module
  (func (export "_start") (loop $l (br $l))))
"#;

/// Asks for 2 GiB of memory and traps if it cannot get it.
const BIG_MEMORY_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "_start")
    (if (i32.eq (memory.grow (i32.const 32768)) (i32.const -1))
      (then (call $deny)))
    (drop (memory.grow (i32.const 0))))
  (func $deny (unreachable)))
"#;

fn run_sync(
    wasm: &[u8],
    argv: &[&str],
    stdin: &[u8],
    limits: WasiLimits,
) -> Result<WasiOutcome, SandboxError> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run_module(
        wasm.to_vec(),
        argv.iter().map(|a| a.to_string()).collect(),
        stdin.to_vec(),
        limits,
    ))
}

#[test]
fn each_stdout_write_reaches_the_observer_in_order() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let sink = seen.clone();
    let observer: StreamObserver =
        std::sync::Arc::new(move |bytes: &[u8]| sink.lock().unwrap().push(bytes.to_vec()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt
        .block_on(run_module_with_mount_streamed(
            TWO_WRITES_WAT.as_bytes().to_vec(),
            vec!["prog".to_string()],
            Vec::new(),
            WasiLimits::default(),
            None,
            Some(observer),
        ))
        .unwrap();
    assert_eq!(out.stdout, b"one\ntwo\n");
    // One chunk per write, in order. Together they are the captured stdout.
    assert_eq!(
        *seen.lock().unwrap(),
        vec![b"one\n".to_vec(), b"two\n".to_vec()]
    );
}

#[test]
fn captures_stdout_and_returns_the_guest_exit_code() {
    let out = run_sync(HELLO_WAT.as_bytes(), &["hello"], b"", WasiLimits::default()).unwrap();
    assert_eq!(out.stdout, b"hi\n");
    assert_eq!(out.exit_code, 3);
}

#[test]
fn runaway_guests_run_out_of_fuel() {
    let limits = WasiLimits {
        fuel: 10_000,
        ..WasiLimits::default()
    };
    let out = run_sync(SPIN_WAT.as_bytes(), &[], b"", limits).unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("trapped"));
}

#[test]
fn memory_growth_past_the_limit_is_refused() {
    let limits = WasiLimits {
        max_memory_bytes: 16 * 1024 * 1024,
        ..WasiLimits::default()
    };
    let out = run_sync(BIG_MEMORY_WAT.as_bytes(), &[], b"", limits).unwrap();
    assert_eq!(out.exit_code, 1);
}

#[test]
fn invalid_modules_are_host_errors() {
    let err = run_sync(b"not wasm", &[], b"", WasiLimits::default()).unwrap_err();
    assert_eq!(err.code, ErrorCode::WasmError);
}

#[test]
fn guest_paths_stay_inside_the_mount() {
    assert_eq!(resolve_guest_path("/work", "a/b").unwrap(), "/work/a/b");
    assert_eq!(resolve_guest_path("/", "a").unwrap(), "/a");
    assert_eq!(resolve_guest_path("/work", "./x").unwrap(), "/work/x");
    assert_eq!(
        resolve_guest_path("/work", "/etc/passwd"),
        Err(errno::NOTCAPABLE)
    );
    assert_eq!(
        resolve_guest_path("/work", "../escape"),
        Err(errno::NOTCAPABLE)
    );
    assert_eq!(
        resolve_guest_path("/work", "a/../../b"),
        Err(errno::NOTCAPABLE)
    );
    assert_eq!(resolve_guest_path("/work", "a\0b"), Err(errno::INVAL));
}

#[test]
fn repeated_runs_reuse_the_compiled_module() {
    // Other tests share the process-wide cache, so check this module's key
    // rather than the cache size.
    let source = HELLO_WAT.as_bytes();
    run_sync(source, &["a"], b"", WasiLimits::default()).unwrap();
    assert!(is_cached(source));
    let out = run_sync(source, &["b"], b"", WasiLimits::default()).unwrap();
    assert!(is_cached(source));
    assert_eq!(out.stdout, b"hi\n");
}
