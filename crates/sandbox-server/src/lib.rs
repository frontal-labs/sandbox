//! API server for the sandbox system.

pub mod api;
pub mod auth;
pub mod gc;
pub mod idle;
pub mod middleware;
pub mod openapi;
pub mod otel;
pub mod persist;
#[cfg(feature = "clustered")]
pub mod pg_store;

pub use api::{
    create_router, create_router_with_options, create_router_with_python, create_router_with_state,
    create_router_with_telemetry, create_router_with_vm, open_durable, open_durable_with_state,
    open_durable_with_vm, AppState, ServerOptions, VmConfig,
};
#[cfg(feature = "clustered")]
pub use api::{open_clustered_with_state, open_clustered_with_vm};
