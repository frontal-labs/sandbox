//! WASI preview1 host for command modules, with files backed by the VFS.
//!
//! The guest sees only what the host gives it:
//! - stdin, stdout, and stderr, all in memory and capped;
//! - argv, and no environment;
//! - optionally, one preopened directory (fd 3, named `.`) that maps to a
//!   directory in the VFS. Guest paths are relative to it. Absolute paths and
//!   `..` are refused with `ENOTCAPABLE`;
//! - CPU limited by fuel, and linear memory capped by `StoreLimits`.
//!
//! No host filesystem, network, or clock beyond wall time is reachable.
//! Calls that the sandbox does not support return `ENOTSUP`, and the rest of
//! preview1 is not provided, so a module that imports it fails to instantiate.

use sandbox_types::hash::ContentHash;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::{DirEntry, FileHandle, FileStat, OpenFlags, SessionFs, Vfs, VfsError};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
};

const WASI: &str = "wasi_snapshot_preview1";
const PREOPEN_FD: u32 = 3;

mod errno {
    pub const SUCCESS: i32 = 0;
    pub const BADF: i32 = 8;
    pub const EXIST: i32 = 20;
    pub const FAULT: i32 = 21;
    pub const INVAL: i32 = 28;
    pub const IO: i32 = 29;
    pub const ISDIR: i32 = 31;
    pub const NOENT: i32 = 44;
    pub const NOSPC: i32 = 51;
    pub const NOTDIR: i32 = 54;
    pub const NOTEMPTY: i32 = 55;
    pub const NOTSUP: i32 = 58;
    pub const PERM: i32 = 63;
    pub const SPIPE: i32 = 70;
    pub const NOTCAPABLE: i32 = 76;
}

/// WASI rights bits the host checks when a file is opened.
const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;
/// `path_open` oflags and fdflags bits.
const OFLAG_CREAT: i32 = 1;
const OFLAG_DIRECTORY: i32 = 2;
const OFLAG_EXCL: i32 = 4;
const OFLAG_TRUNC: i32 = 8;
const FDFLAG_APPEND: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasiLimits {
    /// Instructions the guest may execute. Exhausting the budget ends the run with exit code 1.
    pub fuel: u64,
    /// Maximum captured stdout and stderr, each, in bytes.
    pub max_output_bytes: usize,
    /// Maximum linear memory per module, in bytes.
    pub max_memory_bytes: usize,
}

impl Default for WasiLimits {
    fn default() -> Self {
        Self {
            fuel: 100_000_000,
            max_output_bytes: 1024 * 1024,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasiOutcome {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A VFS directory exposed to the guest as its preopened fd 3.
#[derive(Clone)]
pub struct Mount {
    pub vfs: Arc<Vfs>,
    /// Absolute VFS path of the directory.
    pub root: String,
}

/// Exit requested by `proc_exit`. Travels as a trap and is caught by `execute`.
#[derive(Debug)]
struct Exit(i32);

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "proc_exit({})", self.0)
    }
}

impl std::error::Error for Exit {}

/// The shared engine. Compiled artifacts only load into the engine that
/// produced them, so every run uses this one.
fn engine() -> Result<&'static Engine, SandboxError> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    let built = ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.consume_fuel(true);
        Engine::new(&config).map_err(|e| e.to_string())
    });
    built
        .as_ref()
        .map_err(|e| SandboxError::new(ErrorCode::WasmError, format!("engine setup failed: {e}")))
}

/// Serialized modules this process compiled, keyed by the hash of their source.
///
/// These bytes come only from `Module::serialize` on the shared engine, in
/// this process. They never come from disk or from guest-writable storage,
/// which is why loading them with `Module::deserialize` is sound here.
static MODULE_CACHE: Mutex<BTreeMap<ContentHash, Vec<u8>>> = Mutex::new(BTreeMap::new());
const MODULE_CACHE_CAPACITY: usize = 64;

fn load_module(engine: &Engine, wasm: &[u8]) -> Result<Module, SandboxError> {
    let key = ContentHash::new(wasm);
    let cached = MODULE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
        .cloned();

    if let Some(serialized) = cached {
        // SAFETY: `serialized` was produced by `Module::serialize` on this
        // process's shared engine and is never read from untrusted storage.
        // See the comment on MODULE_CACHE.
        return unsafe { Module::deserialize(engine, &serialized) }
            .map_err(|e| wasm_error("loading cached module failed", e));
    }

    let module = Module::new(engine, wasm)
        .map_err(|e| SandboxError::new(ErrorCode::WasmError, format!("invalid module: {e}")))?;
    let serialized = module
        .serialize()
        .map_err(|e| wasm_error("serializing module failed", e))?;
    let mut cache = MODULE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= MODULE_CACHE_CAPACITY {
        if let Some(evicted) = cache.keys().next().copied() {
            cache.remove(&evicted);
        }
    }
    cache.insert(key, serialized);
    Ok(module)
}

/// Whether this process has a compiled artifact cached for `wasm`.
pub fn is_cached(wasm: &[u8]) -> bool {
    MODULE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(&ContentHash::new(wasm))
}

/// Runs a module with no filesystem access.
pub async fn run_module(
    wasm: Vec<u8>,
    argv: Vec<String>,
    stdin: Vec<u8>,
    limits: WasiLimits,
) -> Result<WasiOutcome, SandboxError> {
    run_module_with_mount(wasm, argv, stdin, limits, None).await
}

/// Runs a module with an optional VFS directory as its preopen.
///
/// The guest runs on a std thread that is entered into the caller's tokio
/// runtime. VFS calls then complete on that thread, so the async VFS API is
/// usable without blocking the runtime's workers.
pub async fn run_module_with_mount(
    wasm: Vec<u8>,
    argv: Vec<String>,
    stdin: Vec<u8>,
    limits: WasiLimits,
    mount: Option<Mount>,
) -> Result<WasiOutcome, SandboxError> {
    run_module_with_mount_streamed(wasm, argv, stdin, limits, mount, None).await
}

/// Like [`run_module_with_mount`], and tells `on_stdout` each piece of stdout as the
/// guest writes it. The pieces are the ones kept in the outcome, in order.
pub async fn run_module_with_mount_streamed(
    wasm: Vec<u8>,
    argv: Vec<String>,
    stdin: Vec<u8>,
    limits: WasiLimits,
    mount: Option<Mount>,
    on_stdout: Option<StreamObserver>,
) -> Result<WasiOutcome, SandboxError> {
    let runtime = tokio::runtime::Handle::current();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        let result = execute(&wasm, argv, stdin, limits, mount, on_stdout);
        // The receiver is only dropped if the caller stopped waiting.
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| SandboxError::new(ErrorCode::WasmError, "wasm worker thread vanished"))?
}

/// Receives the stdout a guest writes, as it is kept, in order. It runs on the
/// guest's thread, so it must not block.
pub type StreamObserver = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Host state for one run. This is the wasmtime store data.
struct Host {
    limits: StoreLimits,
    argv: Vec<String>,
    stdin: Vec<u8>,
    stdin_pos: usize,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    output_cap: usize,
    /// Told each piece of stdout as it is kept. See [`StreamObserver`].
    on_stdout: Option<StreamObserver>,
    mount: Option<Mount>,
    fds: BTreeMap<u32, Fd>,
    next_fd: u32,
}

#[derive(Debug, Clone)]
enum Fd {
    Stdin,
    Stdout,
    Stderr,
    Dir(String),
    File(OpenFile),
}

#[derive(Debug, Clone)]
struct OpenFile {
    path: String,
    handle: FileHandle,
    pos: u64,
    readable: bool,
    writable: bool,
    append: bool,
}

fn execute(
    wasm: &[u8],
    argv: Vec<String>,
    stdin: Vec<u8>,
    limits: WasiLimits,
    mount: Option<Mount>,
    on_stdout: Option<StreamObserver>,
) -> Result<WasiOutcome, SandboxError> {
    let engine = engine()?;
    let module = load_module(engine, wasm)?;

    let mut fds = BTreeMap::new();
    fds.insert(0, Fd::Stdin);
    fds.insert(1, Fd::Stdout);
    fds.insert(2, Fd::Stderr);
    let next_fd = if let Some(mount) = &mount {
        fds.insert(PREOPEN_FD, Fd::Dir(mount.root.clone()));
        PREOPEN_FD + 1
    } else {
        PREOPEN_FD
    };

    let host = Host {
        limits: StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .build(),
        argv,
        stdin,
        stdin_pos: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        output_cap: limits.max_output_bytes,
        on_stdout,
        mount,
        fds,
        next_fd,
    };
    let mut store = Store::new(engine, host);
    store.limiter(|host| &mut host.limits);
    store
        .set_fuel(limits.fuel)
        .map_err(|e| wasm_error("setting fuel failed", e))?;

    let mut linker: Linker<Host> = Linker::new(engine);
    define_host_functions(&mut linker)?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| wasm_error("instantiation failed", e))?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| wasm_error("module has no _start export", e))?;

    let result = start.call(&mut store, ());

    // Release VFS handles the guest left open.
    let host = store.data_mut();
    let open_handles: Vec<FileHandle> = host
        .fds
        .values()
        .filter_map(|fd| match fd {
            Fd::File(file) => Some(file.handle),
            _ => None,
        })
        .collect();
    if let Some(mount) = &host.mount {
        for handle in open_handles {
            mount.vfs.close(&handle);
        }
    }
    let stdout = std::mem::take(&mut host.stdout);
    let stderr = std::mem::take(&mut host.stderr);

    let exit_code = match result {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<Exit>() {
            Some(exit) => exit.0,
            None => {
                let mut stderr = stderr;
                stderr.extend_from_slice(format!("guest trapped: {e}\n").as_bytes());
                return Ok(WasiOutcome {
                    exit_code: 1,
                    stdout,
                    stderr,
                });
            }
        },
    };
    Ok(WasiOutcome {
        exit_code,
        stdout,
        stderr,
    })
}

fn wasm_error(context: &str, error: impl fmt::Display) -> SandboxError {
    SandboxError::new(ErrorCode::WasmError, format!("{context}: {error}"))
}

// --- Guest memory helpers -------------------------------------------------

fn memory(caller: &mut Caller<'_, Host>) -> Result<Memory, i32> {
    match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => Ok(memory),
        _ => Err(errno::FAULT),
    }
}

fn offset(ptr: i32) -> Result<usize, i32> {
    usize::try_from(ptr).map_err(|_| errno::FAULT)
}

fn read_bytes(caller: &mut Caller<'_, Host>, ptr: i32, len: usize) -> Result<Vec<u8>, i32> {
    let mem = memory(caller)?;
    let start = offset(ptr)?;
    let end = start.checked_add(len).ok_or(errno::FAULT)?;
    mem.data(&*caller)
        .get(start..end)
        .map(<[u8]>::to_vec)
        .ok_or(errno::FAULT)
}

fn write_bytes(caller: &mut Caller<'_, Host>, ptr: i32, data: &[u8]) -> Result<(), i32> {
    let mem = memory(caller)?;
    let start = offset(ptr)?;
    mem.write(&mut *caller, start, data)
        .map_err(|_| errno::FAULT)
}

fn read_u32(caller: &mut Caller<'_, Host>, ptr: i32) -> Result<u32, i32> {
    let bytes = read_bytes(caller, ptr, 4)?;
    Ok(u32::from_le_bytes(
        <[u8; 4]>::try_from(bytes.as_slice()).map_err(|_| errno::FAULT)?,
    ))
}

fn write_u8(caller: &mut Caller<'_, Host>, ptr: i32, value: u8) -> Result<(), i32> {
    write_bytes(caller, ptr, &[value])
}

fn write_u32(caller: &mut Caller<'_, Host>, ptr: i32, value: u32) -> Result<(), i32> {
    write_bytes(caller, ptr, &value.to_le_bytes())
}

fn write_u64(caller: &mut Caller<'_, Host>, ptr: i32, value: u64) -> Result<(), i32> {
    write_bytes(caller, ptr, &value.to_le_bytes())
}

fn read_string(caller: &mut Caller<'_, Host>, ptr: i32, len: i32) -> Result<String, i32> {
    let len = usize::try_from(len).map_err(|_| errno::INVAL)?;
    let bytes = read_bytes(caller, ptr, len)?;
    String::from_utf8(bytes).map_err(|_| errno::INVAL)
}

/// Reads `count` iovecs (`buf: u32`, `len: u32` each) at `ptr`.
fn read_iovecs(
    caller: &mut Caller<'_, Host>,
    ptr: i32,
    count: i32,
) -> Result<Vec<(i32, usize)>, i32> {
    let count = usize::try_from(count).map_err(|_| errno::INVAL)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = (offset(ptr)? + i * 8) as i32;
        let buf = read_u32(caller, base)? as i32;
        let len = read_u32(caller, base + 4)? as usize;
        out.push((buf, len));
    }
    Ok(out)
}

// --- Host state helpers ---------------------------------------------------

fn mount_of(caller: &Caller<'_, Host>) -> Result<Mount, i32> {
    caller.data().mount.clone().ok_or(errno::NOTCAPABLE)
}

fn fd_entry(caller: &Caller<'_, Host>, fd: i32) -> Result<Fd, i32> {
    let fd = u32::try_from(fd).map_err(|_| errno::BADF)?;
    caller.data().fds.get(&fd).cloned().ok_or(errno::BADF)
}

fn dir_path(caller: &Caller<'_, Host>, fd: i32) -> Result<String, i32> {
    match fd_entry(caller, fd)? {
        Fd::Dir(path) => Ok(path),
        _ => Err(errno::NOTDIR),
    }
}

fn insert_fd(caller: &mut Caller<'_, Host>, entry: Fd) -> u32 {
    let host = caller.data_mut();
    let fd = host.next_fd;
    host.next_fd += 1;
    host.fds.insert(fd, entry);
    fd
}

/// Joins a guest path onto a mount-relative base. Rejects anything that could
/// leave the mount: absolute paths, `..`, and NUL bytes.
fn resolve_guest_path(base: &str, guest: &str) -> Result<String, i32> {
    if guest.contains('\0') {
        return Err(errno::INVAL);
    }
    if guest.starts_with('/') {
        return Err(errno::NOTCAPABLE);
    }
    let mut path = base.trim_end_matches('/').to_string();
    for part in guest.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(errno::NOTCAPABLE),
            name => {
                path.push('/');
                path.push_str(name);
            }
        }
    }
    if path.is_empty() {
        path.push('/');
    }
    Ok(path)
}

fn errno_of(error: &VfsError) -> i32 {
    match error {
        VfsError::NotFound(_) => errno::NOENT,
        VfsError::AlreadyExists(_) => errno::EXIST,
        VfsError::PermissionDenied(_) => errno::PERM,
        VfsError::QuotaExceeded(_) => errno::NOSPC,
        VfsError::Unsupported(_) => errno::NOTSUP,
        VfsError::InvalidPath(message) => {
            if message.contains("not a directory") {
                errno::NOTDIR
            } else if message.contains("directory not empty") {
                errno::NOTEMPTY
            } else if message.contains("is a directory") || message.contains("regular file") {
                errno::ISDIR
            } else {
                errno::INVAL
            }
        }
        _ => errno::IO,
    }
}

fn file_type(stat: &FileStat) -> u8 {
    if stat.is_dir {
        3
    } else if stat.is_symlink {
        7
    } else if stat.is_file {
        4
    } else {
        0
    }
}

/// `filestat` layout: dev, ino, filetype, nlink, size, atim, mtim, ctim.
fn filestat_bytes(stat: &FileStat) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[16] = file_type(stat);
    out[24..32].copy_from_slice(&1u64.to_le_bytes());
    out[32..40].copy_from_slice(&stat.size.to_le_bytes());
    out
}

// --- Host functions -------------------------------------------------------

/// Turns a `Result<i32, i32>` (value or errno) into the errno return value.
fn finish(result: Result<(), i32>) -> i32 {
    match result {
        Ok(()) => errno::SUCCESS,
        Err(code) => code,
    }
}

fn define_host_functions(linker: &mut Linker<Host>) -> Result<(), SandboxError> {
    let link = |result: wasmtime::Result<&mut Linker<Host>>| {
        result
            .map(|_| ())
            .map_err(|e| wasm_error("defining WASI function failed", e))
    };

    link(linker.func_wrap(
        WASI,
        "args_sizes_get",
        |mut c: Caller<'_, Host>, count_ptr: i32, size_ptr: i32| -> i32 {
            let (count, size) = {
                let argv = &c.data().argv;
                (argv.len(), argv.iter().map(|a| a.len() + 1).sum::<usize>())
            };
            finish(
                write_u32(&mut c, count_ptr, count as u32)
                    .and_then(|()| write_u32(&mut c, size_ptr, size as u32)),
            )
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "args_get",
        |mut c: Caller<'_, Host>, ptrs: i32, buf: i32| -> i32 {
            let argv = c.data().argv.clone();
            let mut cursor = offset(buf).unwrap_or(0);
            let mut result = Ok(());
            for (i, arg) in argv.iter().enumerate() {
                let mut bytes = arg.as_bytes().to_vec();
                bytes.push(0);
                let pointer = cursor as u32;
                result = result
                    .and_then(|()| write_u32(&mut c, ptrs + (i as i32) * 4, pointer))
                    .and_then(|()| write_bytes(&mut c, cursor as i32, &bytes));
                cursor += bytes.len();
            }
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "environ_sizes_get",
        |mut c: Caller<'_, Host>, count_ptr: i32, size_ptr: i32| -> i32 {
            finish(write_u32(&mut c, count_ptr, 0).and_then(|()| write_u32(&mut c, size_ptr, 0)))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "environ_get",
        |_c: Caller<'_, Host>, _ptrs: i32, _buf: i32| -> i32 { errno::SUCCESS },
    ))?;

    link(linker.func_wrap(
        WASI,
        "clock_time_get",
        |mut c: Caller<'_, Host>, _id: i32, _precision: i64, out: i32| -> i32 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            finish(write_u64(&mut c, out, nanos))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "random_get",
        |mut c: Caller<'_, Host>, buf: i32, len: i32| -> i32 {
            let Ok(len) = usize::try_from(len) else {
                return errno::INVAL;
            };
            // Fill with a simple pseudo-random pattern. The sandbox does not
            // provide cryptographic randomness; guests that need it should
            // use a deterministic source or accept this behaviour.
            let mut bytes = vec![0u8; len];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(0x9E)
                    .wrapping_add(0x7D)
                    .wrapping_shr(3);
            }
            finish(write_bytes(&mut c, buf, &bytes))
        },
    ))?;

    link(
        linker.func_wrap(WASI, "sched_yield", |_c: Caller<'_, Host>| -> i32 {
            errno::SUCCESS
        }),
    )?;

    link(linker.func_wrap(
        WASI,
        "proc_exit",
        |_c: Caller<'_, Host>, code: i32| -> wasmtime::Result<()> {
            Err(wasmtime::Error::new(Exit(code)))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_write",
        |mut c: Caller<'_, Host>, fd: i32, iovs: i32, count: i32, out: i32| -> i32 {
            let result = (|| -> Result<u32, i32> {
                let vectors = read_iovecs(&mut c, iovs, count)?;
                let mut data = Vec::new();
                for (buf, len) in vectors {
                    data.extend(read_bytes(&mut c, buf, len)?);
                }
                let entry = fd_entry(&c, fd)?;
                let written = match entry {
                    Fd::Stdout | Fd::Stderr => {
                        let host = c.data_mut();
                        let cap = host.output_cap;
                        let target = if matches!(entry, Fd::Stdout) {
                            &mut host.stdout
                        } else {
                            &mut host.stderr
                        };
                        let room = cap.saturating_sub(target.len());
                        let kept = &data[..data.len().min(room)];
                        target.extend_from_slice(kept);
                        if matches!(entry, Fd::Stdout) {
                            if let Some(observe) = &host.on_stdout {
                                observe(kept);
                            }
                        }
                        // Report the full write so guests do not retry forever.
                        data.len()
                    }
                    Fd::File(file) => {
                        if !file.writable {
                            return Err(errno::BADF);
                        }
                        let mount = mount_of(&c)?;
                        let n = futures::executor::block_on(mount.vfs.write(
                            &file.handle,
                            file.pos,
                            &data,
                        ))
                        .map_err(|e| errno_of(&e))?;
                        update_file(&mut c, fd, |f| f.pos += n as u64);
                        n
                    }
                    Fd::Dir(_) => return Err(errno::ISDIR),
                    Fd::Stdin => return Err(errno::BADF),
                };
                Ok(written as u32)
            })();
            match result {
                Ok(written) => finish(write_u32(&mut c, out, written)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_read",
        |mut c: Caller<'_, Host>, fd: i32, iovs: i32, count: i32, out: i32| -> i32 {
            let result = (|| -> Result<u32, i32> {
                let vectors = read_iovecs(&mut c, iovs, count)?;
                let total: usize = vectors.iter().map(|(_, len)| *len).sum();
                let entry = fd_entry(&c, fd)?;
                let data = match entry {
                    Fd::Stdin => {
                        let host = c.data_mut();
                        let start = host.stdin_pos.min(host.stdin.len());
                        let end = start.saturating_add(total).min(host.stdin.len());
                        host.stdin_pos = end;
                        host.stdin[start..end].to_vec()
                    }
                    Fd::File(file) => {
                        if !file.readable {
                            return Err(errno::BADF);
                        }
                        let mount = mount_of(&c)?;
                        let bytes = futures::executor::block_on(mount.vfs.read(
                            &file.handle,
                            file.pos,
                            total,
                        ))
                        .map_err(|e| errno_of(&e))?;
                        update_file(&mut c, fd, |f| f.pos += bytes.len() as u64);
                        bytes.to_vec()
                    }
                    Fd::Dir(_) => return Err(errno::ISDIR),
                    Fd::Stdout | Fd::Stderr => return Err(errno::BADF),
                };
                let mut cursor = 0;
                for (buf, len) in vectors {
                    if cursor >= data.len() {
                        break;
                    }
                    let n = len.min(data.len() - cursor);
                    write_bytes(&mut c, buf, &data[cursor..cursor + n])?;
                    cursor += n;
                }
                Ok(cursor as u32)
            })();
            match result {
                Ok(read) => finish(write_u32(&mut c, out, read)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_close",
        |mut c: Caller<'_, Host>, fd: i32| -> i32 {
            let Ok(fd) = u32::try_from(fd) else {
                return errno::BADF;
            };
            match c.data_mut().fds.remove(&fd) {
                Some(Fd::File(file)) => {
                    if let Some(mount) = c.data().mount.clone() {
                        mount.vfs.close(&file.handle);
                    }
                    errno::SUCCESS
                }
                Some(_) => errno::SUCCESS,
                None => errno::BADF,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_seek",
        |mut c: Caller<'_, Host>, fd: i32, delta: i64, whence: i32, out: i32| -> i32 {
            let result = (|| -> Result<u64, i32> {
                match fd_entry(&c, fd)? {
                    Fd::File(file) => {
                        let base = match whence {
                            0 => 0i64,
                            1 => file.pos as i64,
                            2 => {
                                let mount = mount_of(&c)?;
                                let stat = futures::executor::block_on(
                                    mount.vfs.stat(std::path::Path::new(&file.path)),
                                )
                                .map_err(|e| errno_of(&e))?;
                                stat.size as i64
                            }
                            _ => return Err(errno::INVAL),
                        };
                        let next = base
                            .checked_add(delta)
                            .filter(|n| *n >= 0)
                            .ok_or(errno::INVAL)? as u64;
                        update_file(&mut c, fd, |f| f.pos = next);
                        Ok(next)
                    }
                    Fd::Dir(_) => Err(errno::ISDIR),
                    Fd::Stdin | Fd::Stdout | Fd::Stderr => Err(errno::SPIPE),
                }
            })();
            match result {
                Ok(pos) => finish(write_u64(&mut c, out, pos)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_tell",
        |mut c: Caller<'_, Host>, fd: i32, out: i32| -> i32 {
            let result = match fd_entry(&c, fd) {
                Ok(Fd::File(file)) => Ok(file.pos),
                Ok(Fd::Dir(_)) => Err(errno::ISDIR),
                Ok(_) => Err(errno::SPIPE),
                Err(code) => Err(code),
            };
            match result {
                Ok(pos) => finish(write_u64(&mut c, out, pos)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_fdstat_get",
        |mut c: Caller<'_, Host>, fd: i32, out: i32| -> i32 {
            let entry = match fd_entry(&c, fd) {
                Ok(entry) => entry,
                Err(code) => return code,
            };
            let (filetype, flags) = match &entry {
                Fd::Stdin | Fd::Stdout | Fd::Stderr => (2u8, 0u16),
                Fd::Dir(_) => (3, 0),
                Fd::File(file) => (4, if file.append { FDFLAG_APPEND as u16 } else { 0 }),
            };
            let mut bytes = [0u8; 24];
            bytes[0] = filetype;
            bytes[2..4].copy_from_slice(&flags.to_le_bytes());
            // Grant every right. The VFS and the open flags enforce access.
            bytes[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
            bytes[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
            finish(write_bytes(&mut c, out, &bytes))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_fdstat_set_flags",
        |c: Caller<'_, Host>, fd: i32, _flags: i32| -> i32 {
            // Flags are fixed at open time. Accept the call, since guests issue it for no-ops.
            match fd_entry(&c, fd) {
                Ok(_) => errno::SUCCESS,
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_filestat_get",
        |mut c: Caller<'_, Host>, fd: i32, out: i32| -> i32 {
            let result = (|| -> Result<[u8; 64], i32> {
                match fd_entry(&c, fd)? {
                    Fd::File(file) => {
                        let mount = mount_of(&c)?;
                        let stat = futures::executor::block_on(
                            mount.vfs.stat(std::path::Path::new(&file.path)),
                        )
                        .map_err(|e| errno_of(&e))?;
                        Ok(filestat_bytes(&stat))
                    }
                    Fd::Dir(path) => {
                        let mount = mount_of(&c)?;
                        let stat = futures::executor::block_on(
                            mount.vfs.stat(std::path::Path::new(&path)),
                        )
                        .map_err(|e| errno_of(&e))?;
                        Ok(filestat_bytes(&stat))
                    }
                    _ => {
                        let mut bytes = [0u8; 64];
                        bytes[16] = 2;
                        bytes[24..32].copy_from_slice(&1u64.to_le_bytes());
                        Ok(bytes)
                    }
                }
            })();
            match result {
                Ok(bytes) => finish(write_bytes(&mut c, out, &bytes)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_filestat_set_size",
        |c: Caller<'_, Host>, fd: i32, size: i64| -> i32 {
            let result = (|| -> Result<(), i32> {
                let file = match fd_entry(&c, fd)? {
                    Fd::File(file) if file.writable => file,
                    Fd::File(_) => return Err(errno::BADF),
                    _ => return Err(errno::INVAL),
                };
                let size = u64::try_from(size).map_err(|_| errno::INVAL)?;
                let mount = mount_of(&c)?;
                futures::executor::block_on(
                    mount.vfs.truncate(std::path::Path::new(&file.path), size),
                )
                .map_err(|e| errno_of(&e))
            })();
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_prestat_get",
        |mut c: Caller<'_, Host>, fd: i32, out: i32| -> i32 {
            let is_preopen = fd == PREOPEN_FD as i32 && c.data().mount.is_some();
            if !is_preopen {
                return errno::BADF;
            }
            finish(write_u8(&mut c, out, 0).and_then(|()| write_u32(&mut c, out + 4, 1)))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_prestat_dir_name",
        |mut c: Caller<'_, Host>, fd: i32, buf: i32, len: i32| -> i32 {
            let is_preopen = fd == PREOPEN_FD as i32 && c.data().mount.is_some();
            if !is_preopen {
                return errno::BADF;
            }
            let name = b".";
            let n = usize::try_from(len).unwrap_or(0).min(name.len());
            finish(write_bytes(&mut c, buf, &name[..n]))
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_open",
        |mut c: Caller<'_, Host>,
         dirfd: i32,
         _lookup: i32,
         path: i32,
         path_len: i32,
         oflags: i32,
         rights: i64,
         _inheriting: i64,
         fdflags: i32,
         out: i32|
         -> i32 {
            let result = (|| -> Result<u32, i32> {
                let base = dir_path(&c, dirfd)?;
                let guest = read_string(&mut c, path, path_len)?;
                let resolved = resolve_guest_path(&base, &guest)?;
                let mount = mount_of(&c)?;
                let rights = rights as u64;
                let readable = rights & RIGHT_FD_READ != 0 || rights & RIGHT_FD_WRITE == 0;
                let writable = rights & RIGHT_FD_WRITE != 0;
                let append = fdflags & FDFLAG_APPEND != 0;
                let create = oflags & OFLAG_CREAT != 0;
                let exclusive = oflags & OFLAG_EXCL != 0;
                let want_dir = oflags & OFLAG_DIRECTORY != 0;

                let existing =
                    futures::executor::block_on(mount.vfs.stat(std::path::Path::new(&resolved)))
                        .ok();
                if let Some(stat) = &existing {
                    if stat.is_dir {
                        if create && exclusive {
                            return Err(errno::EXIST);
                        }
                        if writable {
                            return Err(errno::ISDIR);
                        }
                        return Ok(insert_fd(&mut c, Fd::Dir(resolved)));
                    }
                    if want_dir {
                        return Err(errno::NOTDIR);
                    }
                } else if want_dir {
                    return Err(errno::NOENT);
                }

                let flags = OpenFlags {
                    read: readable,
                    write: writable,
                    append,
                    create,
                    truncate: oflags & OFLAG_TRUNC != 0,
                    exclusive,
                };
                let handle = futures::executor::block_on(
                    mount.vfs.open(std::path::Path::new(&resolved), flags),
                )
                .map_err(|e| errno_of(&e))?;
                let file = OpenFile {
                    path: resolved,
                    handle,
                    pos: 0,
                    readable,
                    writable,
                    append,
                };
                Ok(insert_fd(&mut c, Fd::File(file)))
            })();
            match result {
                Ok(fd) => finish(write_u32(&mut c, out, fd)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_filestat_get",
        |mut c: Caller<'_, Host>,
         dirfd: i32,
         _lookup: i32,
         path: i32,
         path_len: i32,
         out: i32|
         -> i32 {
            let result = (|| -> Result<[u8; 64], i32> {
                let base = dir_path(&c, dirfd)?;
                let guest = read_string(&mut c, path, path_len)?;
                let resolved = resolve_guest_path(&base, &guest)?;
                let mount = mount_of(&c)?;
                let stat =
                    futures::executor::block_on(mount.vfs.stat(std::path::Path::new(&resolved)))
                        .map_err(|e| errno_of(&e))?;
                Ok(filestat_bytes(&stat))
            })();
            match result {
                Ok(bytes) => finish(write_bytes(&mut c, out, &bytes)),
                Err(code) => code,
            }
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_create_directory",
        |mut c: Caller<'_, Host>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let result = (|| -> Result<(), i32> {
                let base = dir_path(&c, dirfd)?;
                let guest = read_string(&mut c, path, path_len)?;
                let resolved = resolve_guest_path(&base, &guest)?;
                let mount = mount_of(&c)?;
                futures::executor::block_on(mount.vfs.mkdir(std::path::Path::new(&resolved), 0o755))
                    .map_err(|e| errno_of(&e))
            })();
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_remove_directory",
        |mut c: Caller<'_, Host>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let result = (|| -> Result<(), i32> {
                let base = dir_path(&c, dirfd)?;
                let guest = read_string(&mut c, path, path_len)?;
                let resolved = resolve_guest_path(&base, &guest)?;
                let mount = mount_of(&c)?;
                let stat =
                    futures::executor::block_on(mount.vfs.stat(std::path::Path::new(&resolved)))
                        .map_err(|e| errno_of(&e))?;
                if !stat.is_dir {
                    return Err(errno::NOTDIR);
                }
                futures::executor::block_on(mount.vfs.rm(std::path::Path::new(&resolved), false))
                    .map_err(|e| errno_of(&e))
            })();
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_unlink_file",
        |mut c: Caller<'_, Host>, dirfd: i32, path: i32, path_len: i32| -> i32 {
            let result = (|| -> Result<(), i32> {
                let base = dir_path(&c, dirfd)?;
                let guest = read_string(&mut c, path, path_len)?;
                let resolved = resolve_guest_path(&base, &guest)?;
                let mount = mount_of(&c)?;
                let stat =
                    futures::executor::block_on(mount.vfs.stat(std::path::Path::new(&resolved)))
                        .map_err(|e| errno_of(&e))?;
                if stat.is_dir {
                    return Err(errno::ISDIR);
                }
                futures::executor::block_on(mount.vfs.rm(std::path::Path::new(&resolved), false))
                    .map_err(|e| errno_of(&e))
            })();
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "path_rename",
        |mut c: Caller<'_, Host>,
         from_fd: i32,
         from: i32,
         from_len: i32,
         to_fd: i32,
         to: i32,
         to_len: i32|
         -> i32 {
            let result = (|| -> Result<(), i32> {
                let from_base = dir_path(&c, from_fd)?;
                let to_base = dir_path(&c, to_fd)?;
                let from_guest = read_string(&mut c, from, from_len)?;
                let to_guest = read_string(&mut c, to, to_len)?;
                let from_path = resolve_guest_path(&from_base, &from_guest)?;
                let to_path = resolve_guest_path(&to_base, &to_guest)?;
                let mount = mount_of(&c)?;
                futures::executor::block_on(mount.vfs.rename(
                    std::path::Path::new(&from_path),
                    std::path::Path::new(&to_path),
                ))
                .map_err(|e| errno_of(&e))
            })();
            finish(result)
        },
    ))?;

    link(linker.func_wrap(
        WASI,
        "fd_readdir",
        |mut c: Caller<'_, Host>, fd: i32, buf: i32, buf_len: i32, cookie: i64, out: i32| -> i32 {
            let result = (|| -> Result<u32, i32> {
                let path = dir_path(&c, fd)?;
                let buf_len = usize::try_from(buf_len).map_err(|_| errno::INVAL)?;
                let mount = mount_of(&c)?;
                let entries: Vec<DirEntry> =
                    futures::executor::block_on(mount.vfs.readdir(std::path::Path::new(&path)))
                        .map_err(|e| errno_of(&e))?;

                // Each record is a 24-byte header followed by the name. The last
                // record may be cut at `buf_len`, which WASI allows.
                let mut data = Vec::new();
                let start = usize::try_from(cookie.max(0)).unwrap_or(usize::MAX);
                for (index, entry) in entries.iter().enumerate().skip(start) {
                    let filetype: u8 = if entry.is_dir {
                        3
                    } else if entry.is_symlink {
                        7
                    } else {
                        4
                    };
                    data.extend_from_slice(&(index as u64 + 1).to_le_bytes());
                    data.extend_from_slice(&0u64.to_le_bytes());
                    data.extend_from_slice(&(entry.name.len() as u32).to_le_bytes());
                    data.push(filetype);
                    data.extend_from_slice(&[0, 0, 0]);
                    data.extend_from_slice(entry.name.as_bytes());
                    if data.len() >= buf_len {
                        break;
                    }
                }
                data.truncate(buf_len);
                write_bytes(&mut c, buf, &data)?;
                Ok(data.len() as u32)
            })();
            match result {
                Ok(used) => finish(write_u32(&mut c, out, used)),
                Err(code) => code,
            }
        },
    ))?;

    link(
        linker.func_wrap(WASI, "fd_sync", |_c: Caller<'_, Host>, _fd: i32| -> i32 {
            errno::SUCCESS
        }),
    )?;
    link(linker.func_wrap(
        WASI,
        "fd_datasync",
        |_c: Caller<'_, Host>, _fd: i32| -> i32 { errno::SUCCESS },
    ))?;

    // Not supported: the guest gets ENOTSUP and the module still instantiates.
    link(linker.func_wrap(
        WASI,
        "poll_oneoff",
        |_c: Caller<'_, Host>, _in: i32, _out: i32, _n: i32, _ret: i32| -> i32 { errno::NOTSUP },
    ))?;
    link(linker.func_wrap(
        WASI,
        "path_readlink",
        |_c: Caller<'_, Host>, _fd: i32, _p: i32, _pl: i32, _b: i32, _bl: i32, _u: i32| -> i32 {
            errno::NOTSUP
        },
    ))?;
    link(linker.func_wrap(
        WASI,
        "path_symlink",
        |_c: Caller<'_, Host>, _a: i32, _al: i32, _fd: i32, _p: i32, _pl: i32| -> i32 {
            errno::NOTSUP
        },
    ))?;

    Ok(())
}

/// Applies `update` to the open file behind `fd`, if it is a file.
fn update_file(caller: &mut Caller<'_, Host>, fd: i32, update: impl FnOnce(&mut OpenFile)) {
    let Ok(fd) = u32::try_from(fd) else {
        return;
    };
    if let Some(Fd::File(file)) = caller.data_mut().fds.get_mut(&fd) {
        update(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
