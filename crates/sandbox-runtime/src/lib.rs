//! V8 runtime host for the sandbox system.
//!
//! `execute_script` runs JavaScript in a fresh isolate per call, with
//! `console.log` captured, a wall-clock timeout, and a `node:` module shim.
//! It runs in the host process. `execute_script_contained` runs through the
//! installed [`ScriptRunner`], and fails closed when none is installed.
//! See [`contain`].

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
