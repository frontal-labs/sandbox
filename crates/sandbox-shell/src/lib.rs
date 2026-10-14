//! Shell parser and interpreter for the sandbox system.
//!
//! The shell runs against a `sandbox_vfs::Vfs` and never touches the host.
//! Supported: simple commands, `|`, `&&`, `||`, `;`, `<`, `>`, `>>`, quotes,
//! `$VAR` and `${VAR}` and `$?`, `NAME=value` assignments, and globs (`*`,
//! `?`, `[...]`) over the VFS. Builtins: `echo pwd cd cat ls mkdir rm mv cp
//! touch head wc true false node wasm`. Other names run as WASI tools found
//! on `PATH` (default `/bin`) in the VFS. Also supported: `if`, `while`,
//! `until`, `for`, `{ }`, subshells, functions, `break`, `continue`, `return`,
//! and `$(...)`. Not supported: `case`, arithmetic, background jobs, fd
//! redirections, and host binaries.

pub mod ast;
pub mod builtins;
pub mod expand;
pub mod interpreter;
pub mod parser;

pub use ast::Script;
pub use interpreter::{ExecutionResult, Shell, ShellConfig};
pub use parser::parse;

#[cfg(test)]
mod tests;
