use axum::{
    routing::{get, post, put},
    Router,
};
use sandbox_scheduler::Scheduler;
use std::sync::Arc;

pub fn create_router(scheduler: Arc<Scheduler>) -> Router {
    Router::new()
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id", axum::routing::delete(destroy_session))
        .route("/v1/sessions/:id/exec", post(exec_command))
        .route("/v1/sessions/:id/fs/*path", get(read_file))
        .route("/v1/sessions/:id/fs/*path", put(write_file))
        .route("/v1/sessions/:id/fs/*path", axum::routing::delete(delete_file))
        .route("/v1/sessions/:id/snapshot", post(take_snapshot))
        .route("/v1/snapshots/:id/fork", post(fork_snapshot))
        .with_state(scheduler)
}

async fn create_session() -> &'static str {
    "create_session"
}

async fn get_session() -> &'static str {
    "get_session"
}

async fn destroy_session() -> &'static str {
    "destroy_session"
}

async fn exec_command() -> &'static str {
    "exec_command"
}

async fn read_file() -> &'static str {
    "read_file"
}

async fn write_file() -> &'static str {
    "write_file"
}

async fn delete_file() -> &'static str {
    "delete_file"
}

async fn take_snapshot() -> &'static str {
    "take_snapshot"
}

async fn fork_snapshot() -> &'static str {
    "fork_snapshot"
}