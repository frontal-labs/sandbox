//! Shell parser and interpreter for the sandbox system.
//!
//! The shell runs against a `sandbox_vfs::Vfs` and never touches the host.
//! Supported: simple commands, `|`, `&&`, `||`, `;`, and `!` on a pipeline;
//! redirections `<`, `>`, `>>`, `>&`, and `2>`, `2>>`, `2>&1`, with `/dev/null`;
//! quotes; `$NAME`, `${NAME}`, `${NAME:-word}`, `${NAME-word}`, `${#NAME}`,
//! `${NAME%pat}`, `${NAME%%pat}`, `${NAME#pat}`, `${NAME##pat}`, `${!NAME}`,
//! `$?`, `$#`, `$@`, `$*`, and `$N`; `$((expr))` integer arithmetic; field
//! splitting on `IFS`; globs over the VFS; `$(...)` and backticks; `if`, `while`,
//! `until`, `for`, `case`, `{ }`, subshells, functions, `break`, `continue`,
//! `return`, and `exit`. Builtins: `echo` (with `-n` and `-e`), `printf`, `read`,
//! `pwd`, `cd`, `cat`, `ls`, `mkdir`, `rm`, `mv`, `cp`, `touch`, `head`, `tail`,
//! `wc`, `grep`, `type`, `test`, `[`, `true`, `false`, `export`, `unset`,
//! `node`, and `wasm`. Other names run as WASI tools found on `PATH` (default
//! `/bin`) in the VFS. Not supported: `set`, `trap`, background jobs, descriptors
//! above 2, and host binaries. Each refusal is an error, not a guess.

pub mod ast;
pub mod builtins;
pub mod coreutils;
mod coreutils_fs;
mod coreutils_text;
pub mod cpu;
pub mod expand;
pub mod interpreter;
pub mod live;
pub mod parser;

mod arith;
mod printf;

pub use ast::Script;
pub use builtins::command_may_read_stdin;
pub use interpreter::{ExecutionResult, OutputChunk, OutputObserver, Shell, ShellConfig};
pub use live::{live_input, LiveInput, LiveSender};
pub use parser::parse;

#[cfg(test)]
mod tests;
