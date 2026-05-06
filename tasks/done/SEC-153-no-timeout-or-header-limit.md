# SEC-153: HTTP server lacks per-request timeout and header-size limit

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/server/routes.rs`
- **Lines**: 1098-1100 (router build, layer chain)

## Description
The router has no `tower_http::timeout::TimeoutLayer`, no slow-loris protection, and no explicit max request-header size beyond axum/hyper defaults. Body limit is set, but a slow body or oversize headers can hold worker threads.

## Recommendation
- Add `TimeoutLayer::new(Duration::from_secs(30))` (or per-route variants for long-poll endpoints).
- Add a header bytes cap via `axum::extract::DefaultBodyLimit::max(...)` companion or `tower_http::limit::RequestBodyLimitLayer` plus `hyper::server::conn::http1::Builder::max_buf_size`.
- Document the limits.

## References
- Related: SEC-094.
