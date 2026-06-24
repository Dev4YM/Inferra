//! HTTP middleware: local security, CSP, and AI rate limits.

use crate::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Instant;
use toml::Value as TomlValue;

pub const CSP_POLICY: &str =
    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";

/// Apply production HTTP middleware (security, CSP, rate limits) to a router.
pub fn apply_http_middleware(state: crate::AppState, router: axum::Router) -> axum::Router {
    router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(axum::middleware::from_fn(csp_middleware))
        .layer(axum::middleware::from_fn_with_state(
            state,
            local_security_middleware,
        ))
}

/// Static UI and root are exempt from loopback/auth checks (matches deprecated Python middleware).
fn is_security_exempt_path(path: &str) -> bool {
    path == "/"
        || path == "/healthz"
        || path == "/readyz"
        || path.starts_with("/assets/")
        || path.starts_with("/static/")
        || path == "/favicon.ico"
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn is_loopback_client(peer: Option<SocketAddr>) -> bool {
    match peer {
        None => true,
        Some(addr) => is_loopback_ip(addr.ip()),
    }
}

fn server_require_loopback(config: &TomlValue) -> bool {
    config
        .get("server")
        .and_then(|s| s.get("require_loopback"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn server_auth_token_env(config: &TomlValue) -> String {
    config
        .get("server")
        .and_then(|s| s.get("auth_token_env"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn server_rate_limit_chat(config: &TomlValue) -> f64 {
    config
        .get("server")
        .and_then(|s| s.get("rate_limit_chat_tokens_per_minute"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(30.0)
        .max(0.0)
}

fn server_rate_limit_explain(config: &TomlValue) -> f64 {
    config
        .get("server")
        .and_then(|s| s.get("rate_limit_explain_tokens_per_minute"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(15.0)
        .max(0.0)
}

fn bearer_matches(request: &Request<Body>, expected: &str) -> bool {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(token) = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
    else {
        return false;
    };
    constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let a = left.get(idx).copied().unwrap_or(0);
        let b = right.get(idx).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    diff == 0
}

fn peer_addr(request: &Request<Body>) -> Option<SocketAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
}

pub async fn local_security_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == axum::http::Method::OPTIONS {
        return next.run(request).await;
    }
    let path = request.uri().path();
    if is_security_exempt_path(path) {
        return next.run(request).await;
    }

    let (require_loopback, auth_env) = {
        let cfg = state.config.read().await;
        (server_require_loopback(&cfg), server_auth_token_env(&cfg))
    };
    if require_loopback && !is_loopback_client(peer_addr(&request)) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"detail": "local clients only"})),
        )
            .into_response();
    }

    if !auth_env.is_empty() {
        let expected = std::env::var(&auth_env)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let Some(expected) = expected else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "detail": format!(
                        "server auth_token_env {:?} is not set in the environment",
                        auth_env
                    )
                })),
            )
                .into_response();
        };
        if !bearer_matches(&request, &expected) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"detail": "unauthorized"})),
            )
                .into_response();
        }
    }

    next.run(request).await
}

pub async fn csp_middleware(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        CSP_POLICY.parse().expect("valid CSP header"),
    );
    response
}

#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    refill_per_second: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_per_second: f64) -> Self {
        Self {
            capacity,
            refill_per_second,
            tokens: capacity,
            last: Instant::now(),
        }
    }

    fn consume(&mut self, cost: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.capacity).min(self.tokens + elapsed * self.refill_per_second);
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
struct HostRateLimiter {
    refill_per_second: f64,
    capacity: f64,
    buckets: HashMap<String, TokenBucket>,
}

impl HostRateLimiter {
    fn new(tokens_per_minute: f64) -> Self {
        let refill_per_second = tokens_per_minute / 60.0;
        let capacity = (8.0_f64).max(refill_per_second * 3.0);
        Self {
            refill_per_second,
            capacity,
            buckets: HashMap::new(),
        }
    }

    fn consume(&mut self, key: &str) -> bool {
        let bucket = self
            .buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_per_second));
        bucket.consume(1.0)
    }
}

pub struct RateLimitState {
    inner: Mutex<RateLimitInner>,
}

struct RateLimitInner {
    chat: HostRateLimiter,
    explain: HostRateLimiter,
    chat_rate: f64,
    explain_rate: f64,
}

impl RateLimitState {
    pub fn new(chat_tokens_per_minute: f64, explain_tokens_per_minute: f64) -> Self {
        Self {
            inner: Mutex::new(RateLimitInner {
                chat: HostRateLimiter::new(chat_tokens_per_minute),
                explain: HostRateLimiter::new(explain_tokens_per_minute),
                chat_rate: chat_tokens_per_minute,
                explain_rate: explain_tokens_per_minute,
            }),
        }
    }

    fn sync_limits(&self, chat_rate: f64, explain_rate: f64) {
        let mut inner = self.inner.lock().expect("rate limit lock");
        if (inner.chat_rate - chat_rate).abs() > f64::EPSILON {
            inner.chat = HostRateLimiter::new(chat_rate);
            inner.chat_rate = chat_rate;
        }
        if (inner.explain_rate - explain_rate).abs() > f64::EPSILON {
            inner.explain = HostRateLimiter::new(explain_rate);
            inner.explain_rate = explain_rate;
        }
    }
}

fn rate_limit_bucket(path: &str) -> Option<&'static str> {
    if path == "/api/ai/ask"
        || path == "/api/ai/investigate-stream"
        || path.starts_with("/api/investigate/")
    {
        return Some("chat");
    }
    if path == "/api/ai/doctor" || path.starts_with("/api/ai/report/") {
        return Some("explain");
    }
    None
}

fn rate_limit_key(peer: Option<SocketAddr>) -> String {
    match peer {
        Some(addr) if is_loopback_ip(addr.ip()) => "loopback".to_string(),
        Some(addr) => addr.ip().to_string(),
        None => "loopback".to_string(),
    }
}

pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let Some(bucket) = rate_limit_bucket(path) else {
        return next.run(request).await;
    };

    let (chat_rate, explain_rate) = {
        let cfg = state.config.read().await;
        (
            server_rate_limit_chat(&cfg),
            server_rate_limit_explain(&cfg),
        )
    };

    state.rate_limits.sync_limits(chat_rate, explain_rate);
    let key = rate_limit_key(peer_addr(&request));
    let allowed = {
        let mut inner = state.rate_limits.inner.lock().expect("rate limit lock");
        match bucket {
            "chat" => inner.chat.consume(&key),
            _ => inner.explain.consume(&key),
        }
    };
    if !allowed {
        let detail = if bucket == "chat" {
            "chat rate limit exceeded"
        } else {
            "explain rate limit exceeded"
        };
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"detail": detail})),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn token_bucket_allows_burst_then_blocks() {
        let mut bucket = TokenBucket::new(2.0, 0.0);
        assert!(bucket.consume(1.0));
        assert!(bucket.consume(1.0));
        assert!(!bucket.consume(1.0));
    }

    #[test]
    fn host_rate_limiter_tracks_keys_independently() {
        let mut limiter = HostRateLimiter::new(600.0);
        assert!(limiter.consume("a"));
        assert!(limiter.consume("b"));
    }

    #[test]
    fn security_exempt_paths_skip_api_routes() {
        assert!(is_security_exempt_path("/"));
        assert!(is_security_exempt_path("/healthz"));
        assert!(is_security_exempt_path("/readyz"));
        assert!(is_security_exempt_path("/assets/app.js"));
        assert!(!is_security_exempt_path("/api/health"));
    }

    #[test]
    fn bearer_comparison_is_exact() {
        assert!(constant_time_eq(b"secret-token", b"secret-token"));
        assert!(!constant_time_eq(b"secret-token", b"secret-tokeo"));
        assert!(!constant_time_eq(b"secret-token", b"secret-token-extra"));
        assert!(!constant_time_eq(b"", b"secret-token"));
    }

    #[test]
    fn rate_limit_routes_classified() {
        assert_eq!(rate_limit_bucket("/api/ai/ask"), Some("chat"));
        assert_eq!(rate_limit_bucket("/api/investigate/now"), Some("chat"));
        assert_eq!(rate_limit_bucket("/api/ai/report/inc-1"), Some("explain"));
        assert_eq!(rate_limit_bucket("/api/overview"), None);
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_loopback_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }
}
