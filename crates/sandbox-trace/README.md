# sandbox-trace

Trace files and replay for the sandbox system. See [ADR 0007](../../docs/adr/0007-trace-format-and-replay.md).

**Status: implemented for the isolate runtime, locally and on the server.** `sandbox run --trace` and `sandbox shell --trace` record scripts, their outcomes, and the files each script changed. The server records each exec the same way (ADRs 0009, 0011). `sandbox replay` re-runs a trace and compares outputs and exit codes.

## Public API

- `format` (`src/format.rs`): `Trace`, `RunRecord` (with its `files: Vec<FileChange>`), `FileChange` (path, op, and content hashes before and after), `FsOp` (a `write` with its bytes, or a `delete`; a run with an `fs` field has an empty script, ADR 0012), `Outcome` (`Completed { exit_code, stdout, stderr }` or `Failed { code, message }`), `TraceError`. A `FileChange` converts from `sandbox_vfs::TreeChange`. See [ADR 0011](../../docs/adr/0011-file-changes-in-traces.md). `Trace::to_jsonl()` writes the file, and `Trace::parse()` reads it. Output bytes are hex-encoded, so they survive exactly.
- `replay` (`src/replay.rs`): `replay(&Trace, run)` calls `run` once per recorded script, in order, and returns a `ReplayReport` (`runs`, `divergences`, `matches()`). Each `Divergence` holds the sequence number, the script, the expected outcome, and the actual outcome. `outcome_of` turns a run result into an `Outcome`. The caller supplies `run`, so this crate does not depend on the session crate.
- `log` (`src/log.rs`): `TraceLog`, an in-memory queue of `sandbox_types::Event` values. Nothing writes to it yet.

## Limits

- Replay checks outputs and exit codes. It does not record or restore the environment, the working directory, or files the trace did not create.
- A script that depends on the clock, randomness, or the host can diverge on replay. That is reported, not hidden.
- Traces hold full output, so noisy scripts make large files.
