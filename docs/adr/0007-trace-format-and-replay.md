# 0007: Trace format and replay

## Status
Accepted

## Date
2026-10-15

## Context
`sandbox replay` returned `Unsupported`, and `sandbox-trace` had a replay stub that reported success without checking anything. The `Event` log in `sandbox-types` records argv and timing, but not the script that ran or what it printed. A replay needs the inputs and the outcomes, so the event log cannot be replayed as it is.

## Decision
- A trace is a JSON Lines file. The first line is the header `{"format":"sandbox-trace","version":1}`. Each later line is one run, in the order it ran:
  - `{"seq":N,"script":"...","exit_code":E,"stdout":"<hex>","stderr":"<hex>"}` when the script completed, including a non-zero exit.
  - `{"seq":N,"script":"...","error":{"code":"...","message":"..."}}` when the script failed before it ran, for example a parse error.
- Stdout and stderr are hex-encoded, so the bytes are exact and the file stays plain text.
- `sandbox run --trace <file>` and `sandbox shell --trace <file>` write a trace of the scripts they run. The trace is written when the command ends.
- `sandbox replay <file>` parses the trace, starts one fresh local session, runs the scripts in order, and compares each outcome with the recorded one. It exits 0 when every run matches, and 1 when any run diverges. Each divergence is printed with its sequence number, the expected outcome, and the actual outcome.
- Replay covers the isolate tier's local session only. The server does not record or replay traces yet.
- A replay is faithful only for scripts whose output does not depend on the clock, randomness, or the host. The format records no environment, so a script that reads the host's state can diverge on replay. That is reported as a divergence, not hidden.
- `sandbox-trace` defines the format and the comparison, and it does not depend on the session crate. The CLI supplies the function that runs one script.

## Consequences
### Positive
- A recorded session can be checked later, and a regression in the shell shows up as a named divergence.
- The format is plain text and needs no new dependency.

### Negative
- Traces hold full output, so a trace of a noisy script can be large.
- The format has no record of the environment, the working directory at each step, or the filesystem contents, so replay checks outputs only. A script that writes a file and reads it back still replays, but a script that depends on files the trace did not create will diverge.
- `ReplayEngine` is removed. Nothing used it.
