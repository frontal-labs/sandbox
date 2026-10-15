# sandbox-trace

Trace files and replay for the sandbox system. See [ADR 0007](../../docs/adr/0007-trace-format-and-replay.md).

**Status: implemented for the local isolate runtime.** `sandbox run --trace` and `sandbox shell --trace` record scripts and their outcomes. `sandbox replay` re-runs them and compares. The server does not record traces yet.

## Public API

- `format` (`src/format.rs`): `Trace`, `RunRecord`, `Outcome` (`Completed { exit_code, stdout, stderr }` or `Failed { code, message }`), `TraceError`. `Trace::to_jsonl()` writes the file, and `Trace::parse()` reads it. Output bytes are hex-encoded, so they survive exactly.
- `replay` (`src/replay.rs`): `replay(&Trace, run)` calls `run` once per recorded script, in order, and returns a `ReplayReport` (`runs`, `divergences`, `matches()`). Each `Divergence` holds the sequence number, the script, the expected outcome, and the actual outcome. `outcome_of` turns a run result into an `Outcome`. The caller supplies `run`, so this crate does not depend on the session crate.
- `log` (`src/log.rs`): `TraceLog`, an in-memory queue of `sandbox_types::Event` values. Nothing writes to it yet.

## Limits

- Replay checks outputs and exit codes. It does not record or restore the environment, the working directory, or files the trace did not create.
- A script that depends on the clock, randomness, or the host can diverge on replay. That is reported, not hidden.
- Traces hold full output, so noisy scripts make large files.
