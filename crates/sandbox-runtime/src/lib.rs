//! V8 runtime host for the sandbox system.
//!
//! `execute_script` runs JavaScript in a fresh isolate per call, with
//! `console.log` captured, a wall-clock timeout, and a `node:` module shim.
//! It runs in the host process. `execute_script_contained` runs through the
//! installed [`ScriptRunner`], and fails closed when none is installed.
//! See [`contain`].
//!
//! The `node:` shim provides `fs` (backed by the session VFS), `env`,
//! `process`, `buffer`, `path`, `url`, `crypto`, `os`, `events`, `stream`,
//! `util`, `assert`, `string_decoder`, and `querystring`. The following are
//! intentionally not implemented in the V8 tier:
//!
//! - **Network** — no `http`, `https`, `net`, `dgram`, or `tls` modules.
//! - **ESM / dynamic `import()`** — only CommonJS `require()` is supported.
//! - **`child_process`** — present as a stub; every method throws.
//! - **`zlib`** — present as a stub; every method throws.
//! - **`crypto.createHash`** — returns a no-op stub (empty digest).

pub mod contain;
pub mod ops;
pub mod runtime;
pub mod typescript;

pub use contain::{
    execute_script_contained, execute_script_contained_with_context,
    install_in_process_script_runner, install_script_runner, InProcessRunner, ScriptRunner,
};
pub use ops::{ScriptContext, StreamObserver};
pub use runtime::{
    execute_script, execute_script_streamed, execute_script_streamed_with_context, ScriptLimits,
    ScriptOutcome,
};
pub use typescript::strip_types;
