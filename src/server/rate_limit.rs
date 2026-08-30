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
//! The limiter is **off unless `SOLIDB_API_RATE_LIMIT` is set**. A database
//! usually sits behind an application tier it trusts, and throttling that tier
//! turns a capacity problem into an availability one. Whoever exposes a node to
//! untrusted clients turns it on.
//!
//! When on, the budget is keyed on the *credential* where a request carries
//! one, and on the address otherwise. Keying on the address alone divides one
//! bucket between every caller that shares an address — several applications on
//! one host, or every client behind a reverse proxy — so each can sit well
//! inside the budget while the total goes past it. `SOLIDB_API_RATE_LIMIT_PER_IP`
//! (default 10x the budget) still caps the address, which is what stops a
//! caller minting credentials to get a fresh bucket per request.
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

/// Max requests per client per window; `0` (the default) disables the limiter.
///
/// Off unless asked for: a database usually sits behind an application tier
/// it trusts, and throttling that tier turns a capacity problem into an
/// availability one — the caller gets a 429 instead of waiting. Whoever
/// exposes a node to untrusted clients knows they have, and turns this on.
static MAX_REQUESTS: Lazy<u32> = Lazy::new(|| {
    std::env::var("SOLIDB_API_RATE_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
});

/// Backstop budget for the *address*, applied on top of the per-credential
/// budget. Defaults to `IP_BACKSTOP_FACTOR` times the per-client budget.
///
/// This is what makes credential keying safe rather than a bypass: the
/// credential is not verified at this layer (verifying it here would duplicate
/// authentication on every request), so a caller can mint a fresh unverifiable
/// credential per request and land in a fresh bucket every time. The address
/// bucket is not forgeable that way — spoofing it means controlling the TCP
/// source — so it catches exactly that spray while staying far enough above
/// legitimate traffic never to fire for a real client.
static MAX_REQUESTS_PER_IP: Lazy<u32> = Lazy::new(|| {
    std::env::var("SOLIDB_API_RATE_LIMIT_PER_IP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| MAX_REQUESTS.saturating_mul(IP_BACKSTOP_FACTOR))
});

/// How much slack the address backstop gets over the per-credential budget.
/// Generous on purpose: it exists to stop credential spraying, not to be the
/// limit anyone runs into.
const IP_BACKSTOP_FACTOR: u32 = 10;

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

/// Stable bucket key for the credential a request carries, if any.
///
/// The credential is hashed, never stored: these keys live in a process-global
/// LRU for the length of a window, and that is no place for a raw API key or
/// JWT. Truncated to 128 bits, which is far past what distinguishing buckets
/// needs.
///
/// Deliberately *not* validated. Validation happens in the auth layer, inside
/// this one; doing it here too would authenticate every request twice. The
/// consequence — an unverifiable credential still gets its own bucket — is
/// what [`MAX_REQUESTS_PER_IP`] exists to contain.
fn credential_key(headers: &HeaderMap) -> Option<String> {
    use sha2::{Digest, Sha256};

    let raw = headers
        .get("X-API-Key")
        .and_then(|h| h.to_str().ok())
        .or_else(|| {
            headers
                .get("Authorization")
                .and_then(|h| h.to_str().ok())
                .and_then(|h| {
                    h.strip_prefix("Bearer ")
                        .or_else(|| h.strip_prefix("ApiKey "))
                })
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    let digest = Sha256::digest(raw.as_bytes());
    Some(format!("cred:{}", hex::encode(&digest[..16])))
}

/// Check and record one request against the client's sliding window.
/// Returns the number of seconds until the budget frees up when it is
/// exhausted, `None` when allowed (or the limiter is disabled).
/// The body of [`check_and_record`], with the budget passed in so a test can
/// exercise a value other than the one this process resolved at startup.
fn check_and_record_against(max_requests: u32, client: &str) -> Option<u64> {
    if max_requests == 0 {
        return None;
    }
    let now = Instant::now();
    let window = Duration::from_secs(*WINDOW_SECS);
    let mut limiter = API_RATE_LIMITER.lock();
    // Fast path first: an existing bucket needs no key allocation, and this
    // runs under a process-global lock on every request.
    if let Some(entry) = limiter.get_mut(client) {
        entry.roll(now, window);
        if entry.estimate(now, window) >= max_requests as f64 {
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

    // Off unless configured — checked before any key work so a node that does
    // not use the limiter pays nothing per request.
    if *MAX_REQUESTS == 0 {
        return next.run(request).await;
    }

    let ip = client_ip(
        peer.ok().map(|axum::extract::ConnectInfo(addr)| addr.ip()),
        request.headers(),
    );
    let credential = credential_key(request.headers());

    // The per-client budget applies to the credential when there is one, so
    // several applications sharing one address — the normal shape behind a
    // proxy, or on a host running an application tier — get a bucket each
    // instead of dividing one between them. The address is then only a
    // backstop against minting credentials to escape that bucket. With no
    // credential the address *is* the client, and carries the full budget.
    let outcome = match &credential {
        Some(key) => check_and_record_against(*MAX_REQUESTS, key).or_else(|| {
            ip.as_ref()
                .and_then(|ip| check_and_record_against(*MAX_REQUESTS_PER_IP, ip))
        }),
        None => match &ip {
            Some(ip) => check_and_record_against(*MAX_REQUESTS, ip),
            // No identifiable client: not throttled, rather than sharing one
            // bucket with every other such caller.
            None => None,
        },
    };

    if let Some(retry_after) = outcome {
        let client = credential.as_deref().or(ip.as_deref()).unwrap_or("unknown");
        tracing::warn!(client = %client, path = %request.uri().path(), "API rate limit exceeded");
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
    fn one_address_two_credentials_get_two_buckets() {
        // The whole point of keying on the credential: several applications on
        // one host, or every client behind a proxy, must not divide one budget
        // between them. A budget of 1 makes it load-bearing.
        let mut app_a = HeaderMap::new();
        app_a.insert("X-API-Key", "key-for-app-a".parse().unwrap());
        let mut app_b = HeaderMap::new();
        app_b.insert("X-API-Key", "key-for-app-b".parse().unwrap());

        let a = credential_key(&app_a).expect("a credential is present");
        let b = credential_key(&app_b).expect("a credential is present");
        assert_ne!(a, b);

        assert!(check_and_record_against(1, &a).is_none());
        assert!(check_and_record_against(1, &b).is_none());
        assert!(check_and_record_against(1, &a).is_some());
    }

    #[test]
    fn the_same_credential_is_always_the_same_bucket() {
        // A per-request key would defeat the limiter as surely as no key.
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", "stable-key".parse().unwrap());
        assert_eq!(credential_key(&headers), credential_key(&headers));
    }

    #[test]
    fn the_credential_never_appears_in_the_bucket_key() {
        // These keys sit in a process-global LRU for a window; a raw API key or
        // JWT must not be what is stored there.
        let secret = "super-secret-api-key";
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", secret.parse().unwrap());
        let key = credential_key(&headers).unwrap();
        assert!(!key.contains(secret));
        assert!(key.starts_with("cred:"));
    }

    #[test]
    fn both_credential_forms_are_recognised() {
        for (name, value) in [
            ("X-API-Key", "k"),
            ("Authorization", "Bearer k"),
            ("Authorization", "ApiKey k"),
        ] {
            let mut h = HeaderMap::new();
            h.insert(name, value.parse().unwrap());
            assert!(credential_key(&h).is_some(), "{name}: {value}");
        }
        // No credential, and a blank one, both fall through to the address.
        assert!(credential_key(&HeaderMap::new()).is_none());
        let mut blank = HeaderMap::new();
        blank.insert("X-API-Key", "".parse().unwrap());
        assert!(credential_key(&blank).is_none());
    }

    #[test]
    fn zero_disables_the_limiter() {
        // What --dev installs. `check_and_record` must never throttle at 0,
        // however many requests one client makes — a dev box runs several
        // applications through one loopback bucket, which is the whole reason
        // the flag turns this off.
        let client = format!("test-zero-{}", std::process::id());
        for _ in 0..5_000 {
            assert!(check_and_record_against(0, &client).is_none());
        }
    }

    #[test]
    fn a_budget_of_zero_is_distinct_from_a_budget_of_one() {
        // Guards the obvious refactor slip: treating 0 as "no requests
        // allowed" rather than "no limit" would lock a dev node out entirely.
        let client = format!("test-one-{}", std::process::id());
        assert!(check_and_record_against(1, &client).is_none());
        assert!(check_and_record_against(1, &client).is_some());
    }

    #[test]
    fn allows_requests_under_the_budget() {
        // An explicit budget, not the process default: the default is now 0
        // (limiter off), which would make this assert nothing.
        let client = format!("test-under-{}", std::process::id());
        for _ in 0..10 {
            assert!(check_and_record_against(20, &client).is_none());
        }
    }

    #[test]
    fn rejects_requests_over_a_tiny_budget() {
        // Use a dedicated bucket and shrink nothing: instead simulate the
        // exhausted state by recording MAX_REQUESTS entries directly.
        let client = format!("test-over-{}", std::process::id());
        let now = Instant::now();
        let mut exhausted = Window::new(now);
        exhausted.current = 5;
        API_RATE_LIMITER.lock().put(client.clone(), exhausted);
        let retry = check_and_record_against(5, &client);
        assert!(retry.is_some());
        assert!(retry.unwrap() >= 1);
    }

    #[test]
    fn distinct_clients_have_distinct_buckets() {
        // A budget of 1 makes the separation load-bearing: if the two shared a
        // bucket the second call would be refused.
        let a = format!("test-a-{}", std::process::id());
        let b = format!("test-b-{}", std::process::id());
        assert!(check_and_record_against(1, &a).is_none());
        assert!(check_and_record_against(1, &b).is_none());
        assert!(check_and_record_against(1, &a).is_some());
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
