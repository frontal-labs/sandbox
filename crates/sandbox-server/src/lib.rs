//! API server for the sandbox system

pub mod api;
pub mod auth;
pub mod middleware;

pub use api::create_router;