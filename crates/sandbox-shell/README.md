# Sandbox Shell

Shell parser and interpreter for the sandbox. It runs against a `sandbox-vfs` `Vfs` and never touches the host.

## Status

Working subset, covered by unit tests in `src/tests.rs` and `src/expand.rs`.

Supported:

- Simple commands, `|` pipelines, `&&`, `||`, `;`, and newlines as separators.
- Compound commands: `if`/`elif`/`else`/`fi`, `while`/`until`/`do`/`done`, `for NAME in WORDS; do ... done`, `{ list; }`, and subshells `( list )`, which run in a copy of the shell state.
- Functions: `name() { ...; }`, called like commands, with positional parameters `$1`..`$9`, `$#`, `$@`, and `$*`. `return N` sets the call's status. Function bodies see the caller's variables.
- Flow control: `break [N]` and `continue [N]`.
- Command substitution: `$(...)` and backticks, nested. Output has trailing newlines removed, and each substitution runs in a copy of the shell state.
- Redirections `<`, `>`, `>>`, with redirect-only commands (`> file` creates the file). File-descriptor redirections (`2>`) are rejected.
- Quoting: single quotes, double quotes, and backslash escapes. Comments with `#`.
- Variables: `$NAME`, `${NAME}`, `$?`, and `NAME=value` assignments. A prefix assignment (`X=1 cmd`) applies only to that command. Expanded values are not globbed or expanded again.
- Globs: `*`, `?`, and `[...]` against the VFS, relative to the working directory. Names starting with `.` match only an explicit `.`. A pattern with no match is kept as written.
- Builtins: `echo pwd cd cat ls mkdir rm mv cp touch head wc true false : test [ break continue return export unset node wasm tools`.
- Tools: a name that isn't a function or builtin is looked up first in the tool registry (`/etc/tools.toml`), then on `PATH` (default `/bin`) in the VFS. The module runs as a WASI command with the working directory mounted as its preopen. See `sandbox-wasm`.

Limits:

- Step budget (`max_steps`, default 1,000,000 commands per run, counting loop iterations and function calls) and output budget (`max_output_bytes`, default 1 MiB). Exceeding either aborts the run.
- Not supported: `case`, `export` of values to host processes, background jobs (`&`), arithmetic `$((...))`, `local`, `set -e`, and host binaries. A name that isn't found exits 127.

## Overview

- `parser.rs`: tokenizer and recursive-descent parser. It keeps a quoting mode for each character and parses `$(...)` into the tree.
- `ast.rs`: the syntax tree: commands, compound commands, words as pieces.
- `expand.rs`: variable and positional expansion, assignment detection, and the glob matcher.
- `interpreter.rs`: pipelines, compound commands, functions, flow control, substitutions, redirections, the glob walk over the VFS, tool lookup, and the budgets.
- `builtins.rs`: builtins implemented against the VFS. `node` runs `sandbox-runtime`, and `wasm` and tool lookup run `sandbox-wasm`.
