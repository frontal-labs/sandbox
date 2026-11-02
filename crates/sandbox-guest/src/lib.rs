//! Guest agent for the sandbox system

pub mod agent;

pub use agent::{start_guest_agent, GuestAgent};

// Re-export the proto types for the guest protocol.
pub mod guest {
    pub use sandbox_proto::v1::guest::*;
}
