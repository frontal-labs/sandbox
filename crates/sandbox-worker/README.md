# Sandbox Worker

Worker process for the sandbox. Each worker runs one session in a child process.

## Status

Working for process isolation only. Covered by unit tests in `src/worker.rs` and by `cli/tests/worker_process.rs`, which runs the real binary.

- `Worker::spawn(program, args)` starts a child that speaks a JSON-lines protocol on stdin and stdout.
- `Worker::run_script(script)` returns the script's `ExecResult`. A script error comes back as an error response, and the worker keeps running.
- A crashed or exited child returns `Unavailable` to the host. The host never panics.
- `serve(session, input, output)` is the child-side loop. `run_stdio_worker` backs the hidden `sandbox worker` subcommand.

Not implemented: seccomp, cgroups, namespaces, `PR_SET_NO_NEW_PRIVS`, and the Linux-only hardening in PLAN.md section 4.13. The worker is not a security boundary against hostile code. No Unix-socket listener and no gRPC service exist. The `tonic`, `prost`, `rustix`, and `nix` dependencies are declared but unused.

## Protocol

One JSON object per line. Requests are `{"op":"run_script","script":"..."}` and `{"op":"shutdown"}`. Each run_script request gets exactly one response line: `{"status":"ok","result":{...}}` or `{"status":"error","code":"...","message":"..."}`.
