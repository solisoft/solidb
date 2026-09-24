//! Inter-node HTTP/WebSocket helpers: URL scheme, timeouts, request signing.
//!
//! Audit L1: every inter-node URL used to be built as `http://` / `ws://`
//! inline, so a cluster whose operator had configured TLS still sent
//! `X-Cluster-Secret` in cleartext. Build peer URLs here instead.
//!
//! Audit A10: the shared client had only a connect timeout, so a peer that
//! accepted the connection and then stalled hung the caller forever. The
//! defaults for the shared client live here too.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Set once at startup when plaintext cannot reach peers at all
/// (`SOLIDB_TLS_REQUIRE=1` with a TLS listener): then `https` is the only
/// scheme that can work, so it becomes the default.
static DEFAULT_HTTPS: AtomicBool = AtomicBool::new(false);

/// Make `https`/`wss` the default when `SOLIDB_CLUSTER_SCHEME` is unset.
pub fn set_default_https(on: bool) {
    DEFAULT_HTTPS.store(on, Ordering::Relaxed);
}

/// The scheme used for inter-node HTTP: `SOLIDB_CLUSTER_SCHEME` (`http` or
/// `https`), else the startup default.
///
/// Anything other than an explicit `http` in the variable means `https`: a
/// typo must not silently downgrade the cluster to cleartext.
pub fn cluster_scheme() -> &'static str {
    match std::env::var("SOLIDB_CLUSTER_SCHEME") {
        Ok(v) if v.trim().eq_ignore_ascii_case("http") => "http",
        Ok(v) if !v.trim().is_empty() => "https",
        _ => {
            if DEFAULT_HTTPS.load(Ordering::Relaxed) {
                "https"
            } else {
                "http"
            }
        }
    }
}

fn ws_scheme_for(http_scheme: &str) -> &'static str {
    if http_scheme == "https" {
        "wss"
    } else {
        "ws"
    }
}

fn join(scheme: &str, addr: &str, path: &str) -> String {
    let sep = if path.starts_with('/') || path.is_empty() {
        ""
    } else {
        "/"
    };
    format!("{}://{}{}{}", scheme, addr, sep, path)
}

/// `scheme://addr/path` for an HTTP request to a peer. An `addr` that already
/// carries a scheme is used as-is (config accepts `https://peer:6745`).
pub fn peer_url(addr: &str, path: &str) -> String {
    peer_url_with(cluster_scheme(), addr, path)
}

/// [`peer_url`] with an explicit scheme (tests, callers that negotiated one).
pub fn peer_url_with(scheme: &str, addr: &str, path: &str) -> String {
    if let Some((given, rest)) = addr.split_once("://") {
        return join(given, rest.trim_end_matches('/'), path);
    }
    join(scheme, addr, path)
}

/// `ws(s)://addr/path` for a WebSocket to a peer, following the HTTP scheme.
pub fn peer_ws_url(addr: &str, path: &str) -> String {
    peer_ws_url_with(cluster_scheme(), addr, path)
}

pub fn peer_ws_url_with(http_scheme: &str, addr: &str, path: &str) -> String {
    if let Some((given, rest)) = addr.split_once("://") {
        let ws = match given {
            "https" | "wss" => "wss",
            _ => "ws",
        };
        return join(ws, rest.trim_end_matches('/'), path);
    }
    join(ws_scheme_for(http_scheme), addr, path)
}

fn env_secs(name: &str, default: u64) -> Duration {
    let secs = std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(default);
    Duration::from_secs(secs)
}

/// Total per-request timeout of the shared inter-node client
/// (`SOLIDB_CLUSTER_HTTP_TIMEOUT_SECS`, default 60).
///
/// Streaming transfers (shard copies, exports) must override it per request
/// with [`stream_timeout`]; `RequestBuilder::timeout` replaces the client's.
pub fn request_timeout() -> Duration {
    env_secs("SOLIDB_CLUSTER_HTTP_TIMEOUT_SECS", 60)
}

/// Idle timeout between reads (`SOLIDB_CLUSTER_HTTP_READ_TIMEOUT_SECS`,
/// default 60). Applies to every request, streaming ones included, so a peer
/// that stops sending mid-stream is still detected.
pub fn read_timeout() -> Duration {
    env_secs("SOLIDB_CLUSTER_HTTP_READ_TIMEOUT_SECS", 60)
}

/// Upper bound for long streaming transfers (`SOLIDB_CLUSTER_STREAM_TIMEOUT_SECS`,
/// default 6 hours). Stalls are caught by [`read_timeout`]; this only caps the
/// total.
pub fn stream_timeout() -> Duration {
    env_secs("SOLIDB_CLUSTER_STREAM_TIMEOUT_SECS", 6 * 3600)
}

/// Headers of a signed inter-node request.
pub const HDR_TS: &str = "X-Cluster-Ts";
pub const HDR_NONCE: &str = "X-Cluster-Nonce";
pub const HDR_SIG: &str = "X-Cluster-Sig";

/// Window within which a signed request is accepted (matches the cluster
/// control-message window in `cluster::transport`).
pub const SIGNED_REQUEST_MAX_SKEW_MS: u64 = 5 * 60 * 1000;

fn request_mac(secret: &str, ts: u64, nonce: &str, method: &str, path_and_query: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(
        format!(
            "{}\n{}\n{}\n{}",
            ts,
            nonce,
            method.to_ascii_uppercase(),
            path_and_query
        )
        .as_bytes(),
    );
    hex::encode(mac.finalize().into_bytes())
}

/// Sign an inter-node request instead of sending the raw cluster secret.
///
/// Returns `(header, value)` pairs to attach. The signature covers the
/// timestamp, a fresh nonce, the method and the path+query, so a captured
/// request cannot be re-aimed at another endpoint, and — once the receiver
/// remembers nonces — cannot be replayed at all.
///
/// Not yet wired into senders: the receiver (`server::auth`) still expects
/// `X-Cluster-Secret`. See [`verify_signed_request`] for the receiver half.
pub fn sign_request(
    secret: &str,
    method: &str,
    path_and_query: &str,
) -> [(&'static str, String); 3] {
    let ts = chrono::Utc::now().timestamp_millis() as u64;
    let nonce = uuid::Uuid::new_v4().to_string();
    let sig = request_mac(secret, ts, &nonce, method, path_and_query);
    [(HDR_TS, ts.to_string()), (HDR_NONCE, nonce), (HDR_SIG, sig)]
}

/// Receiver half of [`sign_request`]: checks window, signature, and that the
/// nonce has not been seen (via the shared replay cache in
/// `cluster::transport`).
pub fn verify_signed_request(
    secret: &str,
    method: &str,
    path_and_query: &str,
    ts: &str,
    nonce: &str,
    sig: &str,
) -> bool {
    if secret.is_empty() || nonce.is_empty() || nonce.len() > 128 {
        return false;
    }
    let Ok(ts) = ts.parse::<u64>() else {
        return false;
    };
    let now = chrono::Utc::now().timestamp_millis() as u64;
    if ts.abs_diff(now) > SIGNED_REQUEST_MAX_SKEW_MS {
        return false;
    }
    let expected = request_mac(secret, ts, nonce, method, path_and_query);
    if !crate::server::auth::constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        return false;
    }
    // Only a correctly signed nonce reaches the cache, so strangers cannot
    // fill it.
    super::transport::remember_nonce(&format!("req:{}", nonce))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_urls_follow_the_scheme() {
        assert_eq!(
            peer_url_with("http", "10.0.0.2:6745", "/_api/x"),
            "http://10.0.0.2:6745/_api/x"
        );
        assert_eq!(
            peer_url_with("https", "10.0.0.2:6745", "_api/x"),
            "https://10.0.0.2:6745/_api/x"
        );
        assert_eq!(
            peer_ws_url_with("https", "n:1", "/_api/ws"),
            "wss://n:1/_api/ws"
        );
        assert_eq!(
            peer_ws_url_with("http", "n:1", "/_api/ws"),
            "ws://n:1/_api/ws"
        );
    }

    #[test]
    fn an_address_with_its_own_scheme_keeps_it() {
        assert_eq!(
            peer_url_with("http", "https://peer:6745/", "/_api/x"),
            "https://peer:6745/_api/x"
        );
        assert_eq!(
            peer_ws_url_with("http", "https://peer:6745", "/ws"),
            "wss://peer:6745/ws"
        );
    }

    const SECRET: &str = "a-shared-cluster-secret-at-least-32-bytes";

    #[test]
    fn a_signed_request_verifies_once() {
        let [(_, ts), (_, nonce), (_, sig)] = sign_request(SECRET, "post", "/_api/a?b=1");
        assert!(verify_signed_request(
            SECRET,
            "POST",
            "/_api/a?b=1",
            &ts,
            &nonce,
            &sig
        ));
        // Replay of the same request is refused.
        assert!(!verify_signed_request(
            SECRET,
            "POST",
            "/_api/a?b=1",
            &ts,
            &nonce,
            &sig
        ));
    }

    #[test]
    fn a_signed_request_cannot_be_re_aimed() {
        let [(_, ts), (_, nonce), (_, sig)] = sign_request(SECRET, "GET", "/_api/a");
        assert!(!verify_signed_request(
            SECRET, "DELETE", "/_api/a", &ts, &nonce, &sig
        ));
        assert!(!verify_signed_request(
            SECRET, "GET", "/_api/b", &ts, &nonce, &sig
        ));
        assert!(!verify_signed_request(
            "other-secret",
            "GET",
            "/_api/a",
            &ts,
            &nonce,
            &sig
        ));
    }

    #[test]
    fn a_stale_signed_request_is_refused() {
        let ts = (chrono::Utc::now().timestamp_millis() as u64) - SIGNED_REQUEST_MAX_SKEW_MS - 1000;
        let nonce = "n-stale";
        let sig = request_mac(SECRET, ts, nonce, "GET", "/x");
        assert!(!verify_signed_request(
            SECRET,
            "GET",
            "/x",
            &ts.to_string(),
            nonce,
            &sig
        ));
    }
}
