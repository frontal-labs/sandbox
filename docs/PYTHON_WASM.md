# Python as a WASI tool

Status: CPython 3.13.1 runs as a registry tool on the sandbox's WASI host. It passes the
conformance cases against the same interpreter built natively. What is still open is
listed at the end.

## Build

`tools/wasm/python/build.sh` builds CPython 3.13.1 for `wasm32-wasip1` in the Linux dev
image, with the WASI SDK 25.0. The source tarball and the SDK are pinned by SHA-256. It
writes to `tools/wasm/python/out/`, which is git-ignored:

| File | Size | Notes |
| --- | --- | --- |
| `python.wasm` | 7.2 MB | stripped |
| `python313.zip` | 14.3 MB | the stdlib without `test/`, stored, not deflated |

The zip is stored because this interpreter has no zlib, so `zipimport` cannot inflate
entries. A WASI build of zlib would make it smaller. The sysroot does not include one.

## Host support

The WASI host gained what CPython needs:

- **Eleven more functions:** `fd_pread` and `fd_pwrite` (positional I/O on files), `fd_advise`
  (checked, then ignored), `clock_res_get` (1 µs, the host clock's resolution), and `path_link`,
  the two timestamp setters and the four socket calls, which return `ENOTSUP`. Hard links need
  inode identity the VFS lacks, timestamps cannot be set, and sockets are not reachable.
- **Read-only preopens.** A tool can be given further directories beside its work
  directory. Each is named by its guest path, such as `/usr`, and mounted read-only. A change
  to anything under it returns `EROFS`, from `path_open` with a write, create or truncate,
  and from create, remove, unlink and rename.
- **Registry fields.** `argv0` replaces the name the shell gave, and `preopen` lists the
  read-only trees. CPython finds its home from `argv0`, because the guest has no environment.

## What runs

With the stdlib zip at `/usr/lib/python313.zip`, mounted read-only at `/usr`, and argv[0]
`/usr/bin/python`:

- `python -c "print(6 * 7)"` prints `42` and exits 0, through the shell and the registry.
- `python -c "open('/usr/lib/python313.zip', 'ab')"` exits 1 with a read-only error.
- The conformance cases in `crates/sandbox-wasm/tests/data/python_cases.json` all match the
  native build's output, with the same exit codes: arithmetic, JSON, regex, `sys.version_info`,
  work-directory file I/O, `os.pread` and `os.pwrite`, `os.listdir`, `sys.exit`, an uncaught
  exception, and comprehensions. A read-only check is included.

Run them with `tools/wasm/python/conformance.sh`. The tests are ignored by default.

## Limits

Measured with the sandbox's counters, under the same host:

- Startup, with `json`, `re` and `os` imported, needs between 1e9 and 3e9 instructions.
  The tool uses **5e9** fuel. A budget of 1e9 traps during the import.
- A 300,000-element list runs under the default 64 MiB. The tool uses **256 MiB**, for real
  work.
- Output uses the default 1 MiB cap.

A registry entry for it:

```toml
[[tool]]
name = "python"
path = "/opt/python.wasm"
argv0 = "/usr/bin/python"
preopen = ["/usr"]
fuel = 5000000000
max_memory_bytes = 268435456
```

The module and the stdlib are not yet installed into a session. The shell test installs them
directly, as a test fixture would.

## Still open

1. **Install into sessions.** The module and the 14 MB stdlib must reach a session's VFS
   before `python` works there. Host-owned files are capped at 64 KiB, so this needs a
   separate path, and it needs a decision on whether every session carries the stdlib.
   Content-addressed storage shares the blob across sessions, so the cost is mainly the
   first copy.
2. **Pin the artifacts.** The build writes hashes, but the registry entry is not in any
   manifest yet. The hashes belong in a committed manifest once artifact storage is decided.
3. **Wizer.** Pre-initialising the interpreter (PLAN.md section 4.6) would cut the startup
   cost. It is an optimisation, and the tool works without it.
4. **Stdlib size.** A subset of the stdlib, without `idlelib`, `tkinter` and `ensurepip`,
   would shrink the zip. The subset needs a list of which modules scripts need.
5. **Performance.** Nothing has been measured on the registry path. It runs under the
   sandbox's host, which is slower than wasmtime alone.
