//! Runs JavaScript in a fresh V8 isolate per call.
//!
//! Each call gets its own isolate, so no heap state carries over between
//! scripts. Wall-clock time is enforced by a watchdog that terminates
//! execution. The heap is capped with V8's heap limits. When the heap nears
//! its limit, a callback terminates the script. Without that callback V8
//! would abort the whole process on out-of-memory.

use crate::ops::{
    env::{op_sandbox_cwd, op_sandbox_env},
    fs::{
        op_fs_mkdir, op_fs_read, op_fs_readdir, op_fs_readlink, op_fs_rename, op_fs_rm, op_fs_stat,
        op_fs_symlink, op_fs_truncate, op_fs_write,
    },
    op_sandbox_print,
    stdio::{op_stderr_write, op_stdout_write},
    time::op_sandbox_now,
    ScriptContext, ScriptOutput, StreamObserver,
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
        op_sandbox_cwd,
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
// The whole bootstrap runs inside one function scope, so its helper bindings never
// become top-level lexical declarations. A user script can then declare `print`,
// `class AssertionError`, and so on without a SyntaxError.
const BOOTSTRAP: &str = r#"
(() => {
const print = Deno.core.ops.op_sandbox_print;
// Decodes UTF-8 bytes. Invalid input yields U+FFFD for the whole run rather than throwing.
function utf8Decode(bytes) {
  let escaped = "";
  for (const b of bytes) escaped += "%" + b.toString(16).padStart(2, "0");
  try { return decodeURIComponent(escaped); } catch { return "�"; }
}
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
  cwd: Deno.core.ops.op_sandbox_cwd,
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
  env: Object.fromEntries(JSON.parse(envOps.get())),
  process: Object.freeze({
    cwd: () => processOps.cwd(),
    env: Object.fromEntries(JSON.parse(envOps.get())),
    nextTick: (fn) => setTimeout(fn, 0),
    hrtime: () => {
      const ns = timeOps.now() * 1000000;
      return [Math.floor(ns / 1e9), ns % 1e9];
    },
    exit: (code) => { throw { code: code || 0, isExit: true }; },
  }),
  child_process: Object.freeze({
    execSync: () => { throw new Error("child_process.execSync is not supported in the sandbox"); },
    spawnSync: () => { throw new Error("child_process.spawnSync is not supported in the sandbox"); },
    spawn: () => { throw new Error("child_process.spawn is not supported in the sandbox"); },
    exec: () => { throw new Error("child_process.exec is not supported in the sandbox"); },
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
  assert: (() => {
    // Structural equality: same prototype, same own keys, recursively equal values.
    const deepEqual = (a, b) => {
      if (Object.is(a, b)) return true;
      if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
      if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
      if (a instanceof Map || a instanceof Set) return deepEqual([...a], [...b]);
      const keys = Object.keys(a);
      if (keys.length !== Object.keys(b).length) return false;
      return keys.every((k) => Object.prototype.hasOwnProperty.call(b, k) && deepEqual(a[k], b[k]));
    };
    const assert = (value, message) => assert.ok(value, message);
    return Object.freeze(Object.assign(assert, {
      ok: (value, message) => { if (!value) throw new AssertionError(message || "Assertion failed"); },
      strict: Object.freeze({
        equal: (a, b, message) => { if (!Object.is(a, b)) throw new AssertionError(message || `expected ${String(a)} to equal ${String(b)}`); },
        deepEqual: (a, b, message) => { if (!deepEqual(a, b)) throw new AssertionError(message || "expected values to be deeply equal"); },
      }),
      throws: (fn, errorMatcher, message) => {
        let threw = false;
        try { fn(); } catch (e) {
          threw = true;
          if (errorMatcher instanceof RegExp && !errorMatcher.test(e.message)) throw new AssertionError(message || "error message did not match");
          if (typeof errorMatcher === "function" && !(e instanceof errorMatcher)) throw new AssertionError(message || "error type did not match");
        }
        if (!threw) throw new AssertionError(message || "expected function to throw");
      },
      doesNotThrow: (fn, message) => {
        try { fn(); } catch (e) { throw new AssertionError(message || `unexpected error: ${e.message}`); }
      },
    }));
  })(),
  zlib: Object.freeze({
    gzipSync: () => { throw new Error("zlib.gzipSync is not supported in the sandbox"); },
    gunzipSync: () => { throw new Error("zlib.gunzipSync is not supported in the sandbox"); },
    deflateSync: () => { throw new Error("zlib.deflateSync is not supported in the sandbox"); },
    inflateSync: () => { throw new Error("zlib.inflateSync is not supported in the sandbox"); },
  }),
  string_decoder: Object.freeze({
    // Only UTF-8 is supported. TextDecoder is not a global here, so bytes are decoded
    // through decodeURIComponent, which is available.
    StringDecoder: class StringDecoder {
      constructor(encoding) {
        const enc = (encoding || "utf8").toLowerCase();
        if (enc !== "utf8" && enc !== "utf-8") throw new Error(`StringDecoder: encoding '${encoding}' is not supported in the sandbox`);
        this._pending = [];
      }
      // Decodes whole characters and keeps an incomplete trailing sequence for the next write.
      write(buf) {
        const bytes = this._pending.concat(Array.from(buf));
        let cut = bytes.length;
        for (let i = bytes.length - 1; i >= Math.max(0, bytes.length - 3); i--) {
          const b = bytes[i];
          if ((b & 0xc0) === 0x80) continue;
          const needed = b >= 0xf0 ? 4 : b >= 0xe0 ? 3 : b >= 0xc0 ? 2 : 1;
          if (bytes.length - i < needed) cut = i;
          break;
        }
        this._pending = bytes.slice(cut);
        return utf8Decode(bytes.slice(0, cut));
      }
      end(buf) {
        const bytes = this._pending.concat(buf ? Array.from(buf) : []);
        this._pending = [];
        return utf8Decode(bytes);
      }
    },
  }),
  querystring: Object.freeze({
    parse: (str, sep, eq) => {
      const result = {};
      const s = str.startsWith("?") ? str.slice(1) : str;
      const sepChar = sep || "&";
      const eqChar = eq || "=";
      // Malformed escapes such as "%E0" are kept as written, not thrown, as in Node.
      const dec = (part) => { try { return decodeURIComponent(part); } catch { return part; } };
      for (const pair of s.split(sepChar)) {
        if (!pair) continue;
        const idx = pair.indexOf(eqChar);
        if (idx === -1) { result[dec(pair)] = ""; }
        else { result[dec(pair.slice(0, idx))] = dec(pair.slice(idx + 1)); }
      }
      return result;
    },
    stringify: (obj, sep, eq) => {
      const sepChar = sep || "&";
      const eqChar = eq || "=";
      return Object.entries(obj)
        .map(([k, v]) => `${encodeURIComponent(k)}${eqChar}${encodeURIComponent(v)}`)
        .join(sepChar);
    },
    escape: encodeURIComponent,
    unescape: decodeURIComponent,
  }),
  module: Object.freeze({
    Module: class Module {
      constructor(id, parent) { this.id = id; this.parent = parent; this.loaded = false; this.exports = {}; }
      static _load() { throw new Error("Module._load is not supported in the sandbox"); }
      static _resolveFilename() { throw new Error("Module._resolveFilename is not supported in the sandbox"); }
    },
  }),
  readline: Object.freeze({
    // Interactive input is not available, so fail loudly rather than answering "" silently.
    createInterface: () => { throw new Error("readline.createInterface is not supported in the sandbox (no stdin)"); },
  }),
  http: Object.freeze({
    createServer: () => { throw new Error("http.createServer is not supported in the sandbox (no network)"); },
    get: () => { throw new Error("http.get is not supported in the sandbox (no network)"); },
    request: () => { throw new Error("http.request is not supported in the sandbox (no network)"); },
  }),
  https: Object.freeze({
    createServer: () => { throw new Error("https.createServer is not supported in the sandbox (no network)"); },
    get: () => { throw new Error("https.get is not supported in the sandbox (no network)"); },
    request: () => { throw new Error("https.request is not supported in the sandbox (no network)"); },
  }),
  net: Object.freeze({
    createServer: () => { throw new Error("net.createServer is not supported in the sandbox (no network)"); },
    connect: () => { throw new Error("net.connect is not supported in the sandbox (no network)"); },
    Socket: class Socket {
      constructor() { throw new Error("net.Socket is not supported in the sandbox (no network)"); }
    },
  }),
  dns: Object.freeze({
    lookup: () => { throw new Error("dns.lookup is not supported in the sandbox (no network)"); },
    resolve: () => { throw new Error("dns.resolve is not supported in the sandbox (no network)"); },
  }),
  tls: Object.freeze({
    createServer: () => { throw new Error("tls.createServer is not supported in the sandbox (no network)"); },
    connect: () => { throw new Error("tls.connect is not supported in the sandbox (no network)"); },
  }),
  worker_threads: Object.freeze({
    get isMainThread() { return true; },
    workerData: null,
    parentPort: null,
    threadId: 0,
    Worker: class Worker {
      constructor() { throw new Error("worker_threads.Worker is not supported in the sandbox"); }
    },
    BroadcastChannel: class BroadcastChannel {
      constructor() { throw new Error("worker_threads.BroadcastChannel is not supported in the sandbox"); }
    },
    MessageChannel: class MessageChannel {
      constructor() { throw new Error("worker_threads.MessageChannel is not supported in the sandbox"); }
    },
    MessagePort: class MessagePort {
      constructor() { throw new Error("worker_threads.MessagePort is not supported in the sandbox"); }
    },
    markAsUntransferable: (obj) => obj,
    receiveMessageOnPort: () => { throw new Error("worker_threads.receiveMessageOnPort is not supported in the sandbox"); },
  }),
});

class AssertionError extends Error {
  constructor(message) {
    super(message);
    this.name = "AssertionError";
  }
}

globalThis.require = (name) => {
  const key = name.startsWith("node:") ? name.slice(5) : name;
  const mod = globalThis.nodeModules[key];
  if (!mod) {
    throw new Error(`Cannot find module '${name}'`);
  }
  return mod;
};

delete globalThis.Deno;
})();
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
    execute_script_streamed_with_context(code, limits, observer, vfs, None).await
}

/// Like [`execute_script_streamed`], but also installs a [`ScriptContext`] (cwd,
/// session env) in the isolate's `OpState`, so the `node:` shim reads the real
/// session values instead of host defaults.
pub async fn execute_script_streamed_with_context(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
    context: Option<ScriptContext>,
) -> Result<ScriptOutcome, SandboxError> {
    let code = code.to_string();
    tokio::task::spawn_blocking(move || {
        // A warm isolate is built for the default heap limit only. Any other limit
        // builds its own isolate, as it always did. A context also forces a fresh
        // build, because the warm isolate was bootstrapped with host defaults.
        if limits.max_heap_bytes == ScriptLimits::default().max_heap_bytes && context.is_none() {
            if let Some(outcome) = run_warm(&code, limits, observer.clone(), vfs.clone()) {
                return outcome;
            }
        }
        run_isolate(&code, limits, observer, vfs, context)
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
    context: Option<ScriptContext>,
) -> ScriptOutcome {
    match build(limits, vfs.clone(), context) {
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
fn build(
    limits: ScriptLimits,
    vfs: Option<Arc<Vfs>>,
    context: Option<ScriptContext>,
) -> Result<Built, String> {
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
    if let Some(context) = context {
        built.runtime.op_state().borrow_mut().put(context);
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
            let Ok(built) = build(ScriptLimits::default(), None, None) else {
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
        // Deno is deleted. `require` is the only node: shim global; `process` is
        // reachable through `require("process")`, not as a global.
        let out = run("console.log(typeof Deno, typeof process, typeof require);").await;
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "undefined undefined function\n"
        );
    }

    #[tokio::test]
    async fn require_resolves_node_modules() {
        let out = run(r#"
            const fs = require("node:fs");
            const path = require("path");
            console.log(typeof fs.readFileSync, typeof path.join);
            "#)
        .await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "function function\n");
    }

    #[tokio::test]
    async fn require_unknown_module_throws() {
        let out = run("require('node:nonexistent')").await;
        assert_eq!(out.exit_code, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot find module"));
    }

    #[tokio::test]
    async fn process_env_starts_empty_without_a_session() {
        // Without a ScriptContext the host environment must not be visible.
        let out = run(r#"
            const env = require("process").env;
            const before = Object.keys(env).length;
            env.FOO = "bar";
            console.log(before, env.FOO, Object.keys(env).length);
            "#)
        .await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "0 bar 1\n");
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
    async fn user_scripts_can_reuse_names_declared_by_the_shim() {
        // The bootstrap's top-level bindings must not collide with a script's own declarations.
        let out = run(r#"
            class AssertionError extends Error {}
            const timeOps = 1;
            console.log(new AssertionError("x") instanceof Error, timeOps);
            "#)
        .await;
        assert_eq!(
            out.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), "true 1\n");
    }

    #[tokio::test]
    async fn node_shim_modules_behave_like_node() {
        let out = run(r#"
            const { StringDecoder } = require("string_decoder");
            const dec = new StringDecoder("utf8");
            const euro = [0xe2, 0x82, 0xac]; // "€" in UTF-8
            const first = dec.write(euro.slice(0, 1));
            const rest = dec.write(euro.slice(1));
            const qs = require("querystring").parse("a=%E0&b=2");
            const assert = require("assert");
            assert({ x: [1, { y: 2 }] }.x.length === 2, "callable");
            assert.strict.deepEqual({ a: [1, 2], b: { c: 3 } }, { a: [1, 2], b: { c: 3 } });
            let threw = false;
            try { require("readline").createInterface({}); } catch (e) { threw = true; }
            console.log(JSON.stringify([first, rest, qs.a, qs.b, threw]));
            "#)
        .await;
        assert_eq!(
            out.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "[\"\",\"€\",\"%E0\",\"2\",true]\n"
        );
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
