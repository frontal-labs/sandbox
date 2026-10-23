//! V8 runtime host for the sandbox system.
//!
//! `execute_script` runs JavaScript in a fresh isolate per call, with
//! `console.log` captured and a wall-clock timeout. It runs in the host process.
//! `execute_script_contained` runs through the installed [`ScriptRunner`], and
//! fails closed when none is installed. See [`contain`]. Filesystem and network
//! access, and modules are not implemented yet.

pub mod contain;
pub mod ops;
pub mod runtime;
pub mod typescript;

pub use contain::{
    execute_script_contained, install_in_process_script_runner, install_script_runner,
    InProcessRunner, ScriptRunner,
};
pub use ops::StreamObserver;
pub use runtime::{execute_script, execute_script_streamed, ScriptLimits, ScriptOutcome};
pub use typescript::strip_types;
