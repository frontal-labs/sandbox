# sandbox-shell

Shell that will run against the VFS with no host OS dependency.

**Status: early prototype.** The parser is whitespace splitting, not `brush-parser`. The interpreter runs one simple command at a time and ignores the VFS it is given. Only five builtins exist (`echo`, `pwd`, `cd`, `true`, `false`), and `echo` and `pwd` write to the host process stdout, not to a captured stream. Pipes, redirects, `&&`, `||`, `;`, control flow, expansion, and command resolution beyond builtins are not implemented. No tests exist yet.

## Public API

Re-exported from `src/lib.rs`: `Shell`, `ShellConfig`, `ExecutionResult`, `parse`, `Builtins`. The modules `ast`, `interpreter`, `parser`, and `process` are public.

- `Shell` (`src/interpreter.rs`)
  - `async new(vfs: Arc<Vfs>, config: ShellConfig) -> Result<Self, SandboxError>`: the VFS is accepted but not stored or used.
  - `async execute(&mut self, input: &str) -> Result<ExecutionResult, SandboxError>`
- `ExecutionResult { exit_code, stdout, stderr }`. `stdout` and `stderr` are always empty. Unknown commands return exit 127 with the message in `stderr`.
- `ShellConfig` (empty struct, `Default`).
- `parse(&str) -> Result<AstNode, SandboxError>` (`src/parser.rs` and `src/interpreter.rs` each define one; the crate root re-exports the interpreter version). It splits on whitespace and produces one `Command`.
- `Builtins` (`src/builtins.rs`): `new`, `get`, `is_builtin`. Registered: `echo`, `pwd` (always prints `/work`), `cd` (prints "not implemented"), `true`, `false`.
- `process` (`src/process.rs`): `VirtualPid`, `Process`, and `ProcessTable`. These are data types with no scheduler behind them.
- `ast` (`src/ast.rs`): AST types for commands, pipelines, redirects, `if`, `while`, `for`, `case`, and function definitions. Only `Command` is produced by the parser.

## Planned behaviour

- Parser: own AST, with `brush-parser` behind it. The crate depends on `brush-parser` 0.2, but the code does not use it yet.
- Resolution order: function, builtin, VFS executable (shebang), WASM tool registry, then `NeedsVm { command }` for promotion.
- Builtins and top-40 coreutils, with GNU-compatible output checked by golden tests.
- Interactive PTY mode is M6.

## Invariants and security assumptions

- No host OS commands should run. Today this holds because no host process is spawned. `echo` and `pwd` do write to host stdout, which should be routed to the session before any multi-tenant use.
- The shell should only touch the VFS. Not yet implemented: the VFS is unused.

## Example

```rust
use std::sync::Arc;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_vfs::Vfs;

async fn run(vfs: Arc<Vfs>) -> Result<(), Box<dyn std::error::Error>> {
    let mut shell = Shell::new(vfs, ShellConfig::default()).await?;
    let result = shell.execute("echo hi").await?;
    assert_eq!(result.exit_code, 0);
    Ok(())
}
```

Not compiled in this pass. Note that `echo hi` prints to host stdout and leaves `result.stdout` empty.

## Spec

See [PLAN.md](../../PLAN.md) section 4.4 and milestone M1. The M1 acceptance criteria include 200+ shell conformance cases, which do not exist yet.
