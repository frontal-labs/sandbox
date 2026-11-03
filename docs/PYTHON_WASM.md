# Python as a WASI tool

Status: the module builds and runs under a reference WASI runtime. It is not yet a registry
tool. The sandbox's WASI host does not provide everything it needs (see Host gaps).

## Build

`tools/wasm/python/build.sh` builds CPython 3.13.1 for `wasm32-wasip1`, in the Linux dev
image, with the WASI SDK 25.0. The source tarball and the SDK are pinned by SHA-256. The script
writes two files to `tools/wasm/python/out/`:

| File | Size | Notes |
| --- | --- | --- |
| `python.wasm` | 7.2 MB | stripped; 28.8 MB before stripping |
| `python313.zip` | 14.3 MB | the stdlib without `test/`, stored, not deflated |

The zip is stored because this interpreter has no zlib: `zipimport` cannot inflate entries.
Compressing it needs a WASI build of zlib, which the sysroot does not include.

## What runs

Under `wasmtime` (Python bindings, 1.x), with `lib/python313.zip` under a preopened `/`:

- `python.wasm -c "print('hello from cpython on wasi', 6*7)"` prints the line and exits 0.
- `import json, re, os` works from the zip.
- Startup plus those imports takes about 1.1 s on this host.

Startup needs no environment variables. With argv[0] set to the bare name `python`, and the
zip at `lib/python313.zip` under the working directory, CPython finds its home. The
`PYTHONHOME` route would need the host to pass an environment, which it does not.

## Host gaps

CPython imports 42 WASI functions. The sandbox host implements about 29 of them. The
missing 13 are:

- Needing real behaviour: `fd_pread`, `fd_pwrite` (positional I/O), `path_link`, and
  `fd_filestat_set_times`, `path_filestat_set_times` (or stub them as unsupported, which
  breaks `os.utime`).
- Stubbable with no effect: `sched_yield` (returns 0), `fd_sync`, `fd_advise` (no-ops on an
  in-memory tree), and `clock_res_get` (a 1 ns resolution).
- Sockets, `sock_accept`, `sock_recv`, `sock_send`, `sock_shutdown`: return ENOTSUP. The
  sandbox has no network access from WASI.

The host also has one preopen (fd 3). The work root needs to stay one preopen. The stdlib
needs a read-only location, so the host must support a second preopen, for example
`/opt/python` read-only, or the stdlib must be copied into each session's work root.

## Still to do before it is a tool

1. Implement the 13 missing functions, with the stubs above, and test them.
2. Add a read-only second preopen, and mount the stdlib zip there.
3. Add a registry entry (`tools.toml`): the pinned BLAKE3 hash, fuel, memory, and output
   limits. The measured startup needs fuel above the default 100M. Measure it with the
   sandbox's counter before choosing a value.
4. Pre-initialise with Wizer (PLAN.md section 4.6), to cut startup, once the host is stable.
5. Conformance tests, against host CPython 3.13.1, for stdout and exit codes.
