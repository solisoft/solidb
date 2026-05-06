# SEC-167: Sync reconnect backoff has no jitter and no global cap

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Lines**: 292-320 (`reconnect_with_backoff`)

## Description
`reconnect_with_backoff` doubles backoff up to 30 s, but the caller in `sync_with_peers` retries every `sync_interval` (1 s) regardless. There is no jitter and no per-peer circuit breaker. A flapping peer triggers a thundering-herd reconnect storm.

## Recommendation
- Add 0–25% random jitter to each backoff step.
- Add a per-peer circuit breaker (e.g., open after 10 consecutive failures, half-open probe every 60 s).

## References
- Related: SEC-110.
