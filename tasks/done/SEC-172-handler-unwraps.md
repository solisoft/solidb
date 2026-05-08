# SEC-172: `unwrap()` on options/results reachable from request handlers

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/server/handlers/sharding.rs`, `src/server/handlers/collections/read.rs`, `src/server/handlers/blobs.rs`
- **Lines**: sharding.rs:58; collections/read.rs:383; blobs.rs:160, 322

## Description
Spot-checked handlers contain `unwrap()` calls reachable from untrusted request flow. A specially crafted request can panic the handler thread.

## Recommendation
Replace each `unwrap()` with `?` against a typed `DbError`, returning a 4xx/5xx as appropriate. Add `clippy::unwrap_used` lint at module boundaries to prevent regressions.

## References
- Related: SEC-094.
