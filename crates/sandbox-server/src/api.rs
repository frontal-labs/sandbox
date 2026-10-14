use crate::auth::{AuthManager, KeyId};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Router,
};
use sandbox_scheduler::Scheduler;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub auth: Arc<AuthManager>,
}

/// Builds the API router. Every `/v1` route requires a valid bearer API key.
///
/// The handlers below are placeholders and answer `501 Not Implemented`. They
/// do not report success for work that was not done.
pub fn create_router(scheduler: Arc<Scheduler>, auth: Arc<AuthManager>) -> Router {
    let state = AppState {
        scheduler,
        auth: auth.clone(),
    };

    Router::new()
        .route("/v1/sessions", post(not_implemented))
        .route("/v1/sessions/:id", get(not_implemented))
        .route("/v1/sessions/:id", delete(not_implemented))
        .route("/v1/sessions/:id/exec", post(not_implemented))
        .route("/v1/sessions/:id/fs/*path", get(not_implemented))
        .route("/v1/sessions/:id/fs/*path", put(not_implemented))
        .route("/v1/sessions/:id/fs/*path", delete(not_implemented))
        .route("/v1/sessions/:id/snapshot", post(not_implemented))
        .route("/v1/snapshots/:id/fork", post(not_implemented))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .with_state(state)
}

/// Rejects requests without a valid `Authorization: Bearer <key>` header.
async fn require_api_key(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let Some(key) = presented else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    match state.auth.validate_api_key(key) {
        Ok(id) => {
            tracing::debug!(key_id = %KeyId::as_str(&id), "request authenticated");
            next.run(request).await
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn not_implemented() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use sandbox_scheduler::Scheduler;
    use tower::ServiceExt;

    fn router(auth: Arc<AuthManager>) -> Router {
        create_router(Arc::new(Scheduler::new()), auth)
    }

    async fn status(app: Router, auth_header: Option<&str>) -> StatusCode {
        let mut builder = HttpRequest::builder().uri("/v1/sessions").method("POST");
        if let Some(value) = auth_header {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let request = builder.body(Body::empty()).unwrap();
        app.oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn missing_key_is_unauthorized() {
        let app = router(Arc::new(AuthManager::new()));
        assert_eq!(status(app, None).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_key_is_unauthorized() {
        let app = router(Arc::new(AuthManager::new()));
        assert_eq!(
            status(app, Some("Bearer sbx_bad_key")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn valid_key_reaches_handler() {
        let auth = Arc::new(AuthManager::new());
        let key = auth.create_key();
        let app = router(auth);
        let header_value = format!("Bearer {key}");
        // Authenticated, but the handler is a placeholder.
        assert_eq!(
            status(app, Some(&header_value)).await,
            StatusCode::NOT_IMPLEMENTED
        );
    }

    #[tokio::test]
    async fn revoked_key_is_unauthorized() {
        let auth = Arc::new(AuthManager::new());
        let key = auth.create_key();
        let id = auth.validate_api_key(&key).unwrap();
        auth.revoke(&id);
        let app = router(auth);
        let header_value = format!("Bearer {key}");
        assert_eq!(
            status(app, Some(&header_value)).await,
            StatusCode::UNAUTHORIZED
        );
    }
}
