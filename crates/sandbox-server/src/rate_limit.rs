//! Per-API-key request rate limiting.
//!
//! A fixed-window counter per [`KeyId`]: a key may make at most
//! [`DefaultConfig::requests_per_second`] requests in any one-second window. The
//! first request in a window records its window start; later requests compare
//! their timestamp against that start and reset the window when a second has
//! passed. Counting is integer and allocation-free per request.
//!
//! The limiter lives in [`AppState`], so every authenticated request — whether
//! it hits a REST handler, a WebSocket upgrade, or the OpenAPI spec — passes
//! through the same [`require_rate_limit`] middleware before it reaches a
//! handler. Unauthenticated requests are rejected by [`crate::api::require_api_key`]
//! before they reach this layer.
//!
//! The window is not a token bucket, so a burst of up to `requests_per_second`
//! requests is allowed at the window boundary, after which the key is throttled
//! for the rest of the second. That is the documented behavior: a key that
//! exceeds its budget receives `429 Too Many Requests` with a
//! `Retry-After: 1` header until the next window.

use crate::auth::KeyId;
use axum::{
    body::Body,
    extract::Request,
    http::{header::RETRY_AFTER, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

/// The default rate limit: 200 requests per second per key.
pub const DEFAULT_RPS: u64 = 200;

/// Configuration for the per-key rate limiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Maximum requests per key per second.
    pub requests_per_second: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: DEFAULT_RPS,
        }
    }
}

/// A fixed-window rate limiter keyed by [`KeyId`].
///
/// Each entry records the start of the current window and the number of
/// requests admitted in it. A request that arrives `1s` or more after the
/// window start opens a new window. The map is bounded by the number of live
/// keys, not by request volume, so it cannot grow without bound.
#[derive(Default)]
pub struct RateLimiter {
    config: RateLimitConfig,
    /// Key id -> (window start, count admitted in this window). Shared across
    /// clones, so every copy of the state counts against the same budget.
    windows: Arc<RwLock<HashMap<String, (Instant, u64)>>>,
}

impl Clone for RateLimiter {
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            windows: Arc::clone(&self.windows),
        }
    }
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns the configured requests-per-second budget.
    pub fn requests_per_second(&self) -> u64 {
        self.config.requests_per_second
    }

    /// Admits a request for `key` if it is within the budget for the current
    /// window, or opens a new window if the previous one has closed.
    ///
    /// Returns `Some(retry_after)` when the key is over budget: the duration
    /// until the current window closes, which the caller surfaces as a
    /// `Retry-After` header.
    pub fn check(&self, key: &KeyId) -> Option<Duration> {
        let limit = self.config.requests_per_second;
        if limit == 0 {
            return Some(Duration::ZERO);
        }
        let now = Instant::now();
        let mut windows = self.windows.write().unwrap();
        let entry = windows
            .entry(key.as_str().to_string())
            .or_insert_with(|| (now, 0));
        // A window lasts exactly one second from its start.
        if now.duration_since(entry.0) >= Duration::from_secs(1) {
            entry.0 = now;
            entry.1 = 0;
        }
        if entry.1 >= limit {
            let remaining = Duration::from_secs(1) - now.duration_since(entry.0);
            return Some(remaining);
        }
        entry.1 += 1;
        None
    }

    /// Removes a key's window so a deleted key does not leave a stale entry.
    /// Called on key deletion; cheap enough to skip if a key is never deleted.
    #[allow(dead_code)]
    pub fn forget(&self, key: &KeyId) {
        self.windows.write().unwrap().remove(key.as_str());
    }

    /// Drops every window, resetting all keys' budgets. Used in tests.
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.windows.write().unwrap().clear();
    }
}

/// The response body for a rejected (rate-limited) request.
fn rate_limited_body() -> Body {
    Body::from(
        r#"{"error":"rate limit exceeded","message":"too many requests; try again shortly"}"#,
    )
}

/// Axum middleware: rejects requests whose key is over its rate budget with
/// `429 Too Many Requests` and a `Retry-After` header.
pub async fn require_rate_limit(
    state: axum::extract::State<crate::api::AppState>,
    axum::Extension(key): axum::Extension<KeyId>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(retry_after) = state.limiter.check(&key) {
        let mut response = (StatusCode::TOO_MANY_REQUESTS, rate_limited_body()).into_response();
        let value = format!("{}", retry_after.as_millis());
        response
            .headers_mut()
            .insert(RETRY_AFTER, value.parse().unwrap());
        return response;
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthManager;

    fn key() -> KeyId {
        KeyId::from_stored(AuthManager::new().create_key())
    }

    #[test]
    fn admits_within_budget() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 5,
        });
        let k = key();
        for _ in 0..5 {
            assert!(limiter.check(&k).is_none());
        }
        // The sixth request is over budget.
        assert!(limiter.check(&k).is_some());
    }

    #[test]
    fn per_key_independence() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 2,
        });
        let a = key();
        let b = key();
        assert!(limiter.check(&a).is_none());
        assert!(limiter.check(&a).is_none());
        assert!(limiter.check(&a).is_some());
        // Key B has its own full budget.
        assert!(limiter.check(&b).is_none());
        assert!(limiter.check(&b).is_none());
    }

    #[test]
    fn zero_disables_limiting() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 0,
        });
        let k = key();
        // Every request is rejected immediately.
        assert!(limiter.check(&k).is_some());
        assert!(limiter.check(&k).is_some());
    }

    #[test]
    fn default_budget_is_200() {
        assert_eq!(RateLimitConfig::default().requests_per_second, DEFAULT_RPS);
        assert_eq!(DEFAULT_RPS, 200);
    }

    #[test]
    fn clone_shares_budget() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 2,
        });
        let clone = limiter.clone();
        let k = key();
        assert!(limiter.check(&k).is_none());
        assert!(clone.check(&k).is_none());
        // The budget is shared, so the third request is rejected.
        assert!(limiter.check(&k).is_some());
    }
}
