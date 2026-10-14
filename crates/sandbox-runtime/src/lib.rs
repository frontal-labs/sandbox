//! V8 runtime host for the sandbox system.
//!
//! `execute_script` runs JavaScript in a fresh isolate per call, with
//! `console.log` captured and a wall-clock timeout. Heap limits, filesystem
//! and network access, and modules are not implemented yet.

pub mod ops;
pub mod runtime;

pub use runtime::{execute_script, ScriptLimits, ScriptOutcome};
