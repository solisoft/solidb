//! General per-client rate limiting for the HTTP API.
//!
//! The login endpoint has its own failed-attempt limiter; this module
//! throttles *all* external API traffic per client IP so one runaway or
//! hostile client cannot monopolize the server (CPU, RocksDB compaction,
//! connection slots). Defaults are deliberately generous — 600 requests / 60s
//! window (a sustained ~10 req/s) — so normal application traffic is never
//! affected; only sustained floods get 429s.
//!
//! Three categories are exempt, and each exemption matters:
//!
//! * **Internal cluster traffic** (a valid `X-Cluster-Secret`). Sharded
//!   writes forward one HTTP request *per document*, and healing/rebalance
//!   add `_batch`, `_replica` and export calls — all from one peer IP, far
//!   above any per-client budget meant for external callers.
//! * **CORS preflights.** An `OPTIONS` never reaches a handler, and counting
//!   it would halve the effective budget for every browser client.
//! * **Requests with no identifiable client** (no `ConnectInfo`, e.g. an
//!   in-process router in tests). Collapsing them into one shared bucket
//!   would throttle unrelated callers against each other.
//!
//! Configuration (environment variables):
//! - `SOLIDB_API_RATE_LIMIT` — max requests per IP per window. `0` disables.
//! - `SOLIDB_API_RATE_WINDOW_SECS` — sliding-window length (default 60).
//!
//! Client identity follows the same rule as the login limiter: the socket
//! peer address, unless `SOLIDB_TRUST_PROXY_HEADERS=1` (behind a proxy that
//! overwrites `X-Forwarded-For`), in which case the forwarded value wins.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use lru::LruCache;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

/// Max requests per client per window; `0` disables the limiter entirely.
static MAX_REQUESTS: Lazy<u32> = Lazy::new(|| {
    std::env::var("SOLIDB_API_RATE_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600)
});

/// Sliding-window length in seconds.
static WINDOW_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("SOLIDB_API_RATE_WINDOW_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(60)
});

/// Bounded LRU of per-client windows. An attacker rotating spoofed IPs can
/// evict other buckets (weakening the limit) but cannot grow the process:
/// each entry is one fixed-size `Window`, so the cache costs the same whether
/// buckets are empty or saturated.
const RATE_LIMITER_CAPACITY: usize = 50_000;

/// Sliding-window counter: the count in the current fixed window plus a
/// time-weighted share of the previous one. Two counters and a timestamp per
/// client, rather than one `Instant` per request — the naive timestamp-vector
/// form costs `MAX_REQUESTS` entries per bucket (~480MB across a full cache at
/// the default budget, unbounded as the budget is raised) and an O(n) sweep
/// under the global lock on every request.
#[derive(Clone, Copy)]
struct Window {
    start: Instant,
    current: u32,
    previous: u32,
}

impl Window {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            current: 0,
            previous: 0,
        }
    }

    /// Roll the window forward to `now`, carrying the immediately preceding
    /// count (and dropping anything older).
    fn roll(&mut self, now: Instant, window: Duration) {
        let elapsed = now.duration_since(self.start);
        if elapsed < window {
            return;
        }
        if elapsed < window * 2 {
            self.previous = self.current;
            self.start += window;
        } else {
            self.previous = 0;
            self.start = now;
        }
        self.current = 0;
    }

    /// Estimated requests in the trailing `window`, weighting the previous
    /// window by how much of it still overlaps.
    fn estimate(&self, now: Instant, window: Duration) -> f64 {
        let elapsed = now.duration_since(self.start).as_secs_f64();
        let overlap = (1.0 - elapsed / window.as_secs_f64()).clamp(0.0, 1.0);
        self.previous as f64 * overlap + self.current as f64
    }
}

static API_RATE_LIMITER: Lazy<Mutex<LruCache<String, Window>>> = Lazy::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(RATE_LIMITER_CAPACITY).unwrap(),
    ))
});

/// Resolve the client identity for a request: socket peer by default, proxy
/// headers when explicitly trusted. `None` when the client cannot be
/// identified at all — such requests are not throttled rather than sharing
/// one bucket.
fn client_ip(peer: Option<std::net::IpAddr>, headers: &HeaderMap) -> Option<String> {
    let socket_ip = peer.map(|ip| ip.to_string());
    if crate::server::auth::trust_proxy_headers() {
        headers
            .get("X-Forwarded-For")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                headers
                    .get("X-Real-IP")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
            })
            .or(socket_ip)
    } else {
        socket_ip
    }
}

/// Check and record one request against the client's sliding window.
/// Returns the number of seconds until the budget frees up when it is
/// exhausted, `None` when allowed (or the limiter is disabled).
fn check_and_record(client: &str) -> Option<u64> {
    if *MAX_REQUESTS == 0 {
        return None;
    }
    let now = Instant::now();
    let window = Duration::from_secs(*WINDOW_SECS);
    let mut limiter = API_RATE_LIMITER.lock();
    // Fast path first: an existing bucket needs no key allocation, and this
    // runs under a process-global lock on every request.
    if let Some(entry) = limiter.get_mut(client) {
        entry.roll(now, window);
        if entry.estimate(now, window) >= *MAX_REQUESTS as f64 {
            // Time until enough of the previous window ages out. Never zero,
            // so a client that honours `Retry-After` always makes progress.
            let remaining = window.saturating_sub(now.duration_since(entry.start));
            return Some(remaining.as_secs() + 1);
        }
        entry.current += 1;
        return None;
    }
    let mut fresh = Window::new(now);
    fresh.current = 1;
    limiter.put(client.to_string(), fresh);
    None
}

/// True when the request carries the cluster keyfile secret, i.e. it is one
/// node talking to another rather than an external client.
fn is_internal_cluster_request(
    state: &crate::server::handlers::AppState,
    headers: &HeaderMap,
) -> bool {
    let provided = match headers
        .get("X-Cluster-Secret")
        .and_then(|h| h.to_str().ok())
    {
        Some(value) if !value.is_empty() => value,
        _ => return false,
    };
    let configured = state
        .storage
        .cluster_config()
        .and_then(|c| c.keyfile.clone())
        .unwrap_or_default();
    if configured.is_empty() {
        return false;
    }
    crate::server::auth::constant_time_eq(configured.as_bytes(), provided.as_bytes())
}

/// Axum middleware: per-client-IP request throttle over the whole router.
pub async fn api_rate_limit_middleware(
    State(state): State<crate::server::handlers::AppState>,
    // `Result` not `Option`: tests build the router without connect info
    // (see the same pattern in `login_handler`).
    peer: Result<
        axum::extract::ConnectInfo<std::net::SocketAddr>,
        axum::extract::rejection::ExtensionRejection,
    >,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS || is_internal_cluster_request(&state, request.headers())
    {
        return next.run(request).await;
    }

    let ip = client_ip(
        peer.ok().map(|axum::extract::ConnectInfo(addr)| addr.ip()),
        request.headers(),
    );
    let Some(ip) = ip else {
        return next.run(request).await;
    };

    if let Some(retry_after) = check_and_record(&ip) {
        tracing::warn!(client = %ip, path = %request.uri().path(), "API rate limit exceeded");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", retry_after.to_string())],
            "Too Many Requests",
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_under_the_budget() {
        let client = format!("test-under-{}", std::process::id());
        for _ in 0..(*MAX_REQUESTS).min(10) {
            assert!(check_and_record(&client).is_none());
        }
    }

    #[test]
    fn rejects_requests_over_a_tiny_budget() {
        // Use a dedicated bucket and shrink nothing: instead simulate the
        // exhausted state by recording MAX_REQUESTS entries directly.
        let client = format!("test-over-{}", std::process::id());
        let now = Instant::now();
        let mut exhausted = Window::new(now);
        exhausted.current = *MAX_REQUESTS;
        API_RATE_LIMITER.lock().put(client.clone(), exhausted);
        let retry = check_and_record(&client);
        assert!(retry.is_some());
        assert!(retry.unwrap() >= 1);
    }

    #[test]
    fn distinct_clients_have_distinct_buckets() {
        let a = format!("test-a-{}", std::process::id());
        let b = format!("test-b-{}", std::process::id());
        assert!(check_and_record(&a).is_none());
        assert!(check_and_record(&b).is_none());
    }

    #[test]
    fn unidentifiable_clients_are_not_throttled() {
        // No ConnectInfo and no trusted proxy headers: the caller cannot be
        // named, so it must not share a bucket with everyone else.
        assert!(client_ip(None, &HeaderMap::new()).is_none());
    }

    #[test]
    fn window_carries_the_previous_count_then_forgets_it() {
        let window = Duration::from_secs(60);
        let start = Instant::now();
        let mut w = Window::new(start);
        w.current = 100;

        // One window later: the previous count carries, fully weighted.
        let next = start + window;
        w.roll(next, window);
        assert_eq!(w.previous, 100);
        assert_eq!(w.current, 0);
        assert!((w.estimate(next, window) - 100.0).abs() < 1.0);

        // Halfway through the new window it counts for half.
        let mid = next + window / 2;
        assert!((w.estimate(mid, window) - 50.0).abs() < 1.0);

        // Two windows of silence and it is gone entirely.
        let later = next + window * 3;
        w.roll(later, window);
        assert_eq!(w.previous, 0);
        assert_eq!(w.estimate(later, window), 0.0);
    }
}
