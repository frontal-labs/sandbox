# Sandbox Worker

Worker process for the sandbox. Each worker runs one session in a child process.

## Status

Working for process isolation. Covered by unit tests in `src/worker.rs` and integration tests in `cli/tests/worker_process.rs`, which run the real `sandbox` binary as a worker.

- The child serves `sandbox.v1.worker.WorkerService` (gRPC, from `sandbox-proto`) on a Unix socket in a directory the host creates with mode `0700`. The socket path is passed with `sandbox worker --socket <path>`.
- `Worker::spawn(program)` starts the child and waits for `Health`. `run_script` and `exec` return the same `ExecResult` the session returns. A script error keeps its own `ErrorCode`.
- Requests and responses are limited to 16 MiB on both sides. Larger scripts are refused on the host with `ResourceExhausted`. Each call has a 60 s deadline.
- A dead or unreachable child returns `Unavailable`. Dropping the `Worker` kills the child and removes its socket directory. `shutdown` asks the child to exit first.
- Guest output is data in the `ExecResult`, not a protocol channel. The old stdout JSON-lines protocol is gone.

Not implemented: seccomp, cgroups, namespaces, and `PR_SET_NO_NEW_PRIVS` (Linux-only; see PLAN.md section 4.13). The worker isolates crashes, not hostile code. No scheduler client uses this yet.
