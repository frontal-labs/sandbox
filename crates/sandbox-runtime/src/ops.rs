//! Ops exposed to scripts. Each op is a host capability, so adding one is a
//! security decision. There is no filesystem, network, or environment access
//! yet, so the only op is the one that writes script output.

use deno_core::{op2, OpState};

/// Output captured for one script run. Stored in `OpState` for the run.
#[derive(Default)]
pub struct ScriptOutput {
    pub stdout: Vec<u8>,
    /// Maximum bytes kept in `stdout`. Anything beyond it is dropped.
    pub limit: usize,
    pub truncated: bool,
}

impl ScriptOutput {
    pub fn push(&mut self, text: &str) {
        let room = self.limit.saturating_sub(self.stdout.len());
        if text.len() <= room {
            self.stdout.extend_from_slice(text.as_bytes());
        } else {
            self.stdout.extend_from_slice(&text.as_bytes()[..room]);
            self.truncated = true;
        }
    }
}

/// Appends text to the captured stdout. Backs `console.log`.
#[op2(fast)]
pub fn op_sandbox_print(state: &mut OpState, #[string] text: String) {
    if let Some(output) = state.try_borrow_mut::<ScriptOutput>() {
        output.push(&text);
    }
}
