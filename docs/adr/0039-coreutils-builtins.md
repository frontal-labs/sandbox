# 0039: Coreutils builtins over the VFS

## Status
Accepted

## Date
2026-11-10

## Context
PLAN §4.4 lists the coreutils the shell needs and asks for GNU golden tests. Most of the list was missing, and the ones that existed (`ls`, `cat`, `cp`, `mv`, `rm`, `mkdir`, `touch`, `head`, `tail`, `wc`, `grep`) had no golden tests. Two options were open: port the uutils crates, which need an abstract filesystem trait they do not expose, or implement each command against the VFS. PLAN §4.4 allows the second when uutils does not fit.

## Decision
- Commands are implemented in `sandbox-shell` against `SessionFs`, in three modules: `coreutils.rs` (text and path utilities), `coreutils_fs.rs` (`find`, `xargs`, `du`, `stat`, `realpath`), and `coreutils_text.rs` (`sed`, `diff`, `nl`, `base64`, `date`).
- Each command supports the options listed in its doc comment. An option it does not support is an error, not silent. A script therefore never gets different output without an error.
- Golden tests: `conformance/coreutils_cases.py` holds the cases. `record_coreutils.py` runs them under GNU coreutils 9.11, sed, gawk, grep, findutils, diffutils, gnu-tar, and gzip from Homebrew, with their `gnubin` directories first on PATH. The result is `conformance/coreutils.json`. `tests/coreutils_conformance.rs` replays it and reports every mismatch.
- The bash corpus (`cases.json`) is now recorded under the same GNU PATH. It had been recorded with macOS's BSD tools, which disagree with GNU on `wc` padding and on the `ls` exit status. Ten cases changed, and each change is a GNU behaviour the sandbox now matches.
- `command_may_read_stdin` lists the builtins that read stdin, so the live-stdin WebSocket route (ADR 0038) waits for input for them.

## Known divergences from GNU
- `du` counts directories as zero bytes. GNU counts their block size. Plain files match.
- `realpath` without `--relative-to` prints the sandbox path. GNU prints the host path, which is not comparable, so the goldens use `--relative-to` only.
- `date` supports only `-d @SECONDS` and `-u`. The sandbox has one time zone, UTC.
- `find` visits entries in the VFS's sorted order. GNU uses readdir order, which it does not promise. Goldens pipe through `sort`.
- `sed` supports only the `s` command with the `g`, `p`, and `i` flags, and `-n` and `-E`. BRE is translated to the `regex` crate's syntax, so features the translation does not cover are refused by the regex parser.
- `diff` supports only the normal format and two files.
- `wc` prints one count bare, and pads several counts to eight columns, as GNU does.

## Consequences
### Positive
- Every supported option has a GNU-recorded expectation, and a change to the output of a command shows up in the replay.
- No new crate dependency.

### Negative
- The golden recording needs the Homebrew GNU tools. CI does not install them yet, so the golden replay runs from the committed `coreutils.json`. Re-recording needs a machine with the tools.

## Alternatives Considered
- Port uutils. Not chosen for now: the crates expect a host filesystem trait, and the VFS would need an adapter for each one.
- Record goldens on the host's own tools. Not chosen: macOS's BSD tools disagree with GNU, and that is what the earlier corpus did wrong.

## References
- PLAN §4.4, §7, §9 M2. ADR 0019 (shell conformance), ADR 0038 (live stdin).
