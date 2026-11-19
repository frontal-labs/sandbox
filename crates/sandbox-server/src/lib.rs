//! API server for the sandbox system.

pub mod api;
pub mod auth;
pub mod billing;
pub mod config;
pub mod gc;
pub mod idle;
pub mod metering;
pub mod middleware;
pub mod openapi;
pub mod otel;
pub mod parked;
pub mod persist;
#[cfg(feature = "clustered")]
pub mod pg_store;
pub mod pty;

pub use api::{
    create_router, create_router_with_options, create_router_with_python, create_router_with_state,
    create_router_with_telemetry, create_router_with_vm, AppState, ServerOptions, VmConfig,
};
#[cfg(not(feature = "clustered"))]
pub use api::{
    open_durable, open_durable_with_options, open_durable_with_state, open_durable_with_vm,
};
#[cfg(feature = "clustered")]
pub use api::{open_clustered_with_remote, open_clustered_with_state, open_clustered_with_vm};
pub use config::{load as load_config, ConfigError, ResolvedServeConfig};
