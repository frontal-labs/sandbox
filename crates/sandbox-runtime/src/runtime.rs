//! Runs JavaScript in a fresh V8 isolate per call.
//!
//! Each call gets its own isolate, so no heap state carries over between
//! scripts. Wall-clock time is enforced by a watchdog that terminates
//! execution. The heap is capped with V8's heap limits. When the heap nears
//! its limit, a callback terminates the script. Without that callback V8
//! would abort the whole process on out-of-memory.

use crate::ops::{
    env::op_sandbox_env,
    fs::{
        op_fs_mkdir, op_fs_read, op_fs_readdir, op_fs_readlink, op_fs_rename, op_fs_rm, op_fs_stat,
        op_fs_symlink, op_fs_truncate, op_fs_write,
    },
    op_sandbox_print,
    stdio::{op_stderr_write, op_stdout_write},
    time::op_sandbox_now,
    ScriptOutput, StreamObserver,
};
use deno_core::{extension, JsRuntime, RuntimeOptions};
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::Vfs;
use std::sync::{mpsc, Arc};
use std::time::Duration;

extension!(
    sandbox_script,
    ops = [
        op_sandbox_print,
        op_sandbox_env,
        op_sandbox_now,
        op_fs_read,
        op_fs_write,
        op_fs_stat,
        op_fs_readdir,
        op_fs_mkdir,
        op_fs_rm,
        op_fs_rename,
        op_fs_symlink,
        op_fs_readlink,
        op_fs_truncate,
        op_stdout_write,
        op_stderr_write,
    ]
);

/// Installed before each script. Defines `console.log` on top of the print op
/// and provides the `node:` module shim.
///
/// It keeps the ops it needs and then deletes the `Deno` global. The
/// default deno_core ops (such as `op_print`, which writes to the host's
/// stdout) would otherwise let scripts forge worker protocol responses.
const BOOTSTRAP: &str = r#"
const print = Deno.core.ops.op_sandbox_print;
globalThis.console = Object.freeze({
  log: (...args) => {
    const text = args.map((a) => {
      if (typeof a === "string") return a;
      try { return JSON.stringify(a) ?? String(a); } catch { return String(a); }
    }).join(" ");
    print(text + "\n");
  },
});

// node: module shim
const fsOps = {
  read: Deno.core.ops.op_fs_read,
  write: Deno.core.ops.op_fs_write,
  stat: Deno.core.ops.op_fs_stat,
  readdir: Deno.core.ops.op_fs_readdir,
  mkdir: Deno.core.ops.op_fs_mkdir,
  rm: Deno.core.ops.op_fs_rm,
  rename: Deno.core.ops.op_fs_rename,
  symlink: Deno.core.ops.op_fs_symlink,
  readlink: Deno.core.ops.op_fs_readlink,
  truncate: Deno.core.ops.op_fs_truncate,
};

const envOps = {
  get: Deno.core.ops.op_sandbox_env,
};

const timeOps = {
  now: Deno.core.ops.op_sandbox_now,
};

const processOps = {
  spawn: Deno.core.ops.op_spawn,
};

const stdioOps = {
  stdoutWrite: Deno.core.ops.op_stdout_write,
  stderrWrite: Deno.core.ops.op_stderr_write,
};

// Install node: modules
globalThis.nodeModules = Object.freeze({
  fs: Object.freeze({
    readFileSync: (path) => {
      const result = fsOps.read(path);
      return new Uint8Array(result);
    },
    writeFileSync: (path, data) => {
      fsOps.write(path, data);
    },
    statSync: (path) => {
      const result = fsOps.stat(path);
      return {
        isFile: () => result.isFile,
        isDirectory: () => result.isDirectory,
        isSymbolicLink: () => result.isSymlink,
        size: result.size,
        mode: result.mode,
        mtimeMs: result.mtime,
      };
    },
    readdirSync: (path) => {
      return fsOps.readdir(path);
    },
    mkdirSync: (path, options) => {
      const mode = options?.mode ? parseInt(options.mode, 8) : 0o755;
      fsOps.mkdir(path, mode.toString(8));
    },
    rmSync: (path, options) => {
      fsOps.rm(path, !!options?.recursive);
    },
    renameSync: (oldPath, newPath) => {
      fsOps.rename(oldPath, newPath);
    },
    symlinkSync: (target, path) => {
      fsOps.symlink(target, path);
    },
    readlinkSync: (path) => {
      return fsOps.readlink(path);
    },
    truncateSync: (path, len) => {
      fsOps.truncate(path, len);
    },
  }),
  env: Object.freeze({
    get env() {
      return envOps.get();
    },
  }),
  process: Object.freeze({
    cwd: () => "/",
    env: {},
    nextTick: (fn) => setTimeout(fn, 0),
    hrtime: () => {
      const ns = timeOps.now() * 1000000;
      return [Math.floor(ns / 1e9), ns % 1e9];
    },
    exit: (code) => { throw { code: code || 0, isExit: true }; },
  }),
  child_process: Object.freeze({
    execSync: (command) => {
      const result = processOps.spawn(command);
      if (result.exitCode !== 0) {
        throw new Error(result.stderr);
      }
      return new TextEncoder().encode(result.stdout);
    },
    spawnSync: (command, args, options) => {
      const fullCommand = [command, ...(args || [])].join(" ");
      const result = processOps.spawn(fullCommand);
      return {
        status: result.exitCode,
        stdout: new TextEncoder().encode(result.stdout),
        stderr: new TextEncoder().encode(result.stderr),
      };
    },
  }),
  buffer: Object.freeze({
    Buffer: class Buffer extends Uint8Array {
      static from(data) {
        if (typeof data === "string") {
          return new TextEncoder().encode(data);
        }
        return new Uint8Array(data);
      }
    },
  }),
  path: Object.freeze({
    join: (...args) => args.join("/").replace(/\/+/g, "/"),
    resolve: (...args) => args.join("/").replace(/\/+/g, "/"),
    dirname: (path) => path.split("/").slice(0, -1).join("/") || "/",
    basename: (path) => path.split("/").pop() || "",
    extname: (path) => {
      const base = path.split("/").pop() || "";
      const dot = base.lastIndexOf(".");
      return dot === -1 ? "" : base.slice(dot);
    },
    sep: "/",
  }),
  url: Object.freeze({
    URL: globalThis.URL,
    URLSearchParams: globalThis.URLSearchParams,
  }),
  crypto: Object.freeze({
    randomBytes: (size) => {
      const arr = new Uint8Array(size);
      crypto.getRandomValues(arr);
      return arr;
    },
    randomUUID: () => crypto.randomUUID(),
    createHash: (algorithm) => {
      return {
        update: (data) => { /* no-op for now */ },
        digest: () => new Uint8Array(32),
      };
    },
  }),
  os: Object.freeze({
    platform: () => "linux",
    arch: () => "x64",
    cpus: () => [],
    totalmem: () => 0,
    freemem: () => 0,
    hostname: () => "sandbox",
    homedir: () => "/",
    tmpdir: () => "/tmp",
  }),
  events: Object.freeze({
    EventEmitter: class EventEmitter {
      constructor() { this._events = {}; }
      on(event, listener) { this._events[event] = this._events[event] || []; this._events[event].push(listener); return this; }
      emit(event, ...args) { (this._events[event] || []).forEach(l => l(...args)); }
      off(event, listener) { this._events[event] = (this._events[event] || []).filter(l => l !== listener); }
    },
  }),
  stream: Object.freeze({
    Readable: class Readable {
      constructor() { this._readable = true; }
      pipe(dest) { return dest; }
    },
    Writable: class Writable {
      constructor() { this._writable = true; }
      write(chunk) { stdioOps.stdoutWrite(new TextEncoder().encode(chunk)); }
      end() {}
    },
    Duplex: class Duplex {},
    Transform: class Transform {},
  }),
  util: Object.freeze({
    inspect: (obj) => JSON.stringify(obj),
    promisify: (fn) => (...args) => Promise.resolve(fn(...args)),
  }),
});

delete globalThis.Deno;
"#;

#[derive(Debug, Clone, Copy)]
pub struct ScriptLimits {
    /// Wall-clock budget for the script.
    pub timeout: Duration,
    /// Maximum captured stdout, in bytes.
    pub max_output_bytes: usize,
    /// Maximum V8 heap, in bytes.
    pub max_heap_bytes: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            max_output_bytes: 1024 * 1024,
            max_heap_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptOutcome {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `code` as a classic script in a fresh isolate.
///
/// A thrown error or a timeout gives exit code 1 and a message on stderr.
/// Those are script outcomes, not host errors, so they come back as `Ok`.
pub async fn execute_script(
    code: &str,
    limits: ScriptLimits,
) -> Result<ScriptOutcome, SandboxError> {
    execute_script_streamed(code, limits, None, None).await
}

/// Like [`execute_script`], and tells `observer` each piece of stdout as the script
/// writes it. The pieces are the ones kept in the outcome, in order.
///
/// # Containment limits
///
/// This runs the script in the host process, with no OS-level containment. The heap
/// limit terminates the script at `max_heap_bytes`, but it does not bound all memory:
/// string growth and off-heap buffers can exceed it, and a single allocation past the
/// limit is a fatal V8 out-of-memory that aborts the host. The `node` builtin does not
/// use this path. It goes through [`crate::execute_script_contained`], which runs each
/// script in a child process with an OS memory limit.
pub async fn execute_script_streamed(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> Result<ScriptOutcome, SandboxError> {
    let code = code.to_string();
    tokio::task::spawn_blocking(move || {
        // A warm isolate is built for the default heap limit only. Any other limit
        // builds its own isolate, as it always did.
        if limits.max_heap_bytes == ScriptLimits::default().max_heap_bytes {
            if let Some(outcome) = run_warm(&code, limits, observer.clone(), vfs.clone()) {
                return outcome;
            }
        }
        run_isolate(&code, limits, observer, vfs)
    })
    .await
    .map_err(|e| SandboxError::new(ErrorCode::RuntimeError, format!("script task failed: {e}")))
}

/// Extra heap V8 may use after the near-limit callback fires, so the script
/// can unwind after termination.
const HEAP_HEADROOM: usize = 16 * 1024 * 1024;
/// How many times the callback grants `HEAP_HEADROOM` while a script is terminating.
const MAX_HEADROOM_GRANTS: usize = 4;
struct HeapGuard {
    /// Terminates the script when the heap limit is reached. A thread-safe handle,
    /// so it stays valid as long as the guard does.
    handle: deno_core::v8::IsolateHandle,
    hit: std::sync::atomic::AtomicBool,
    /// Headroom grants so far. Capped at `MAX_HEADROOM_GRANTS`, so the heap cannot
    /// grow without bound while termination is pending.
    grants: std::sync::atomic::AtomicUsize,
}

/// Called by V8 when the heap is close to its limit.
///
/// # Safety
///
/// `data` must point to a live `HeapGuard`. `Built` guarantees this: its runtime
/// is declared before its guard, so the isolate is gone before the guard is freed.
unsafe extern "C" fn on_near_heap_limit(
    data: *mut std::ffi::c_void,
    current: usize,
    _initial: usize,
) -> usize {
    // SAFETY: see the function's safety contract; `data` points into the `Box`
    // owned by the `Built` that installed this callback.
    let guard = unsafe { &*(data as *const HeapGuard) };
    guard.hit.store(true, std::sync::atomic::Ordering::SeqCst);
    guard.handle.terminate_execution();
    let granted = guard
        .grants
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if granted >= MAX_HEADROOM_GRANTS {
        // Termination is still pending and the grants are spent. V8 may now fail the
        // allocation fatally. That is the one case in-process isolation cannot contain.
        return current;
    }
    current + HEAP_HEADROOM
}

/// Runs `code` in a freshly built isolate. Used when no warm isolate is ready.
fn run_isolate(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> ScriptOutcome {
    match build(limits, vfs.clone()) {
        Ok(built) => run_built(built, code, limits, observer, vfs),
        Err(message) => failed(message),
    }
}

/// An isolate with its bootstrap done and its heap guard installed, ready for one script.
/// It lives on the thread that built it, because V8 isolates are not `Send`.
///
/// Fields drop in declaration order, so the isolate goes before the guard that its
/// near-heap-limit callback reads through a raw pointer.
struct Built {
    runtime: JsRuntime,
    /// A `Box` keeps the guard at a fixed address when `Built` moves.
    guard: Box<HeapGuard>,
}

/// Builds an isolate with the heap limit of `limits`, the heap guard, and the bootstrap.
fn build(limits: ScriptLimits, vfs: Option<Arc<Vfs>>) -> Result<Built, String> {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![sandbox_script::init()],
        create_params: Some(
            deno_core::v8::CreateParams::default().heap_limits(0, limits.max_heap_bytes),
        ),
        ..RuntimeOptions::default()
    });

    let guard = Box::new(HeapGuard {
        handle: runtime.v8_isolate().thread_safe_handle(),
        hit: std::sync::atomic::AtomicBool::new(false),
        grants: std::sync::atomic::AtomicUsize::new(0),
    });
    let guard_ptr = std::ptr::from_ref::<HeapGuard>(&guard)
        .cast_mut()
        .cast::<std::ffi::c_void>();
    runtime
        .v8_isolate()
        .add_near_heap_limit_callback(on_near_heap_limit, guard_ptr);
    let mut built = Built { runtime, guard };
    if let Some(vfs) = vfs {
        built.runtime.op_state().borrow_mut().put(vfs);
    }
    if let Err(e) = built
        .runtime
        .execute_script("[sandbox:bootstrap]", BOOTSTRAP)
    {
        return Err(format!("bootstrap failed: {e}"));
    }
    Ok(built)
}

/// Runs `code` in a built isolate, under the wall-clock limit of `limits`.
fn run_built(
    mut built: Built,
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> ScriptOutcome {
    let runtime = &mut built.runtime;
    runtime.op_state().borrow_mut().put(ScriptOutput {
        limit: limits.max_output_bytes,
        observer,
        ..ScriptOutput::default()
    });
    if let Some(vfs) = vfs {
        runtime.op_state().borrow_mut().put(vfs);
    }

    // Watchdog: terminate the isolate if the deadline passes first. The
    // channel lets a finished script stop the watchdog.
    let handle = runtime.v8_isolate().thread_safe_handle();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog_flag = timed_out.clone();
    let timeout = limits.timeout;
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(timeout).is_err() {
            watchdog_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            handle.terminate_execution();
        }
    });

    let result = runtime.execute_script("[sandbox:script]", code.to_string());
    let _ = done_tx.send(());
    let _ = watchdog.join();

    let output = runtime.op_state().borrow_mut().take::<ScriptOutput>();
    let heap_hit = built.guard.hit.load(std::sync::atomic::Ordering::SeqCst);
    drop(built);
    let (stdout, truncated) = (output.stdout, output.truncated);

    let mut outcome = match result {
        Ok(_) => ScriptOutcome {
            exit_code: 0,
            stdout,
            stderr: Vec::new(),
        },
        Err(_) if heap_hit => {
            let mut out = failed(format!(
                "script exceeded the heap limit of {} bytes",
                limits.max_heap_bytes
            ));
            out.stdout = stdout;
            out
        }
        Err(_) if timed_out.load(std::sync::atomic::Ordering::SeqCst) => {
            let mut out = failed(format!("script timed out after {:?}", limits.timeout));
            out.stdout = stdout;
            out
        }
        Err(e) => {
            let mut out = failed(e.to_string());
            out.stdout = stdout;
            out
        }
    };
    if truncated {
        outcome.stderr.extend_from_slice(b"[output truncated]\n");
    }
    outcome
}

/// Isolates built ahead of need. A `node` run takes one, so it starts from a ready
/// isolate rather than building one (PLAN §4.14, warm pools). The pool holds
/// `WARM_TARGET` at most, and each taken isolate is replaced in the background.
const WARM_TARGET: usize = 2;

struct WarmJob {
    code: String,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
    reply: mpsc::Sender<ScriptOutcome>,
}

#[derive(Default)]
struct WarmPool {
    /// Threads whose isolate is built and waiting for a job.
    ready: std::sync::Mutex<Vec<mpsc::Sender<WarmJob>>>,
}

static WARM: std::sync::OnceLock<std::sync::Arc<WarmPool>> = std::sync::OnceLock::new();

fn warm_pool() -> &'static std::sync::Arc<WarmPool> {
    WARM.get_or_init(|| {
        let pool = std::sync::Arc::new(WarmPool::default());
        for _ in 0..WARM_TARGET {
            spawn_warm(pool.clone());
        }
        pool
    })
}

/// Starts a thread that builds one isolate, offers it to the pool, and runs the one
/// job it is given. The isolate is single-use, so the thread then ends.
fn spawn_warm(pool: std::sync::Arc<WarmPool>) {
    let spawned = std::thread::Builder::new()
        .name("sandbox-warm-isolate".to_string())
        .spawn(move || {
            // V8 posts delayed tasks (heap-limit handling) to tokio's timer, so the
            // isolate must be built inside a tokio runtime context. A plain thread has none.
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let _entered = rt.enter();
            let Ok(built) = build(ScriptLimits::default(), None) else {
                return;
            };
            let (offer, jobs) = mpsc::channel::<WarmJob>();
            if let Ok(mut ready) = pool.ready.lock() {
                ready.push(offer);
            }
            if let Ok(job) = jobs.recv() {
                let outcome = run_built(built, &job.code, job.limits, job.observer, job.vfs);
                let _ = job.reply.send(outcome);
            }
        });
    // A thread that cannot be started leaves the pool smaller. Runs then build their own.
    let _ = spawned;
}

/// Runs `code` in a warm isolate, if one is ready. `None` means none was, and the
/// caller builds its own isolate.
fn run_warm(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> Option<ScriptOutcome> {
    let pool = warm_pool();
    let offer = pool.ready.lock().ok()?.pop()?;
    // Refill in the background, so the pool is back to its target size.
    spawn_warm(pool.clone());
    let (reply, outcome) = mpsc::channel();
    offer
        .send(WarmJob {
            code: code.to_string(),
            limits,
            observer,
            vfs,
            reply,
        })
        .ok()?;
    outcome.recv().ok()
}

fn failed(message: String) -> ScriptOutcome {
    ScriptOutcome {
        exit_code: 1,
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(code: &str) -> ScriptOutcome {
        execute_script(code, ScriptLimits::default()).await.unwrap()
    }

    #[tokio::test]
    async fn console_log_is_captured() {
        let out = run(r#"console.log("hello", 1 + 2, {a: 1});"#).await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello 3 {\"a\":1}\n");
    }

    #[tokio::test]
    async fn thrown_errors_are_script_failures_not_host_errors() {
        let out = run("throw new Error('boom');").await;
        assert_eq!(out.exit_code, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("boom"));
    }

    #[tokio::test]
    async fn infinite_loops_are_terminated_by_the_watchdog() {
        let limits = ScriptLimits {
            timeout: Duration::from_millis(200),
            ..ScriptLimits::default()
        };
        let out = execute_script("while (true) {}", limits).await.unwrap();
        assert_eq!(out.exit_code, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
    }

    #[tokio::test]
    async fn output_is_capped() {
        let limits = ScriptLimits {
            max_output_bytes: 10,
            ..ScriptLimits::default()
        };
        let out = execute_script(
            "for (let i = 0; i < 100; i++) console.log('abcdef');",
            limits,
        )
        .await
        .unwrap();
        assert_eq!(out.stdout.len(), 10);
        assert!(String::from_utf8_lossy(&out.stderr).contains("truncated"));
    }

    #[tokio::test]
    async fn each_run_starts_with_fresh_state() {
        run("globalThis.leak = 42;").await;
        let out = run("console.log(typeof globalThis.leak);").await;
        assert_eq!(String::from_utf8_lossy(&out.stdout), "undefined\n");
    }

    #[tokio::test]
    async fn host_filesystem_and_environment_are_not_reachable() {
        let out = run("console.log(typeof Deno, typeof process, typeof require);").await;
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "undefined undefined undefined\n"
        );
    }

    #[tokio::test]
    async fn default_deno_ops_are_unreachable_from_scripts() {
        // Attempts to reach the default ops (including op_print, which writes to
        // host stdout) must fail. Nothing should be written to the host stream.
        let out = run("try { globalThis.Deno.core.print('forged\\n', false); } catch (e) { console.log('blocked'); }").await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "blocked\n");
    }

    #[tokio::test]
    async fn heap_growth_is_stopped_before_it_aborts_the_process() {
        let limits = ScriptLimits {
            max_heap_bytes: 32 * 1024 * 1024,
            timeout: Duration::from_secs(20),
            ..ScriptLimits::default()
        };
        let out = execute_script(
            "const a = []; while (true) a.push({ x: 'y'.repeat(1000) });",
            limits,
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("heap limit"));
    }
}
