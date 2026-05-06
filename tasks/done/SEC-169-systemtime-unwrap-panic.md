# SEC-169: `SystemTime::duration_since(UNIX_EPOCH).unwrap()` panics on clock skew

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/server/auth.rs`, `src/sync/transport.rs`
- **Lines**: auth.rs:618, 644; transport.rs:534

## Description
Several timestamps are taken via `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()`. If the system clock ever runs before 1970 (NTP failure, container misconfiguration), the unwrap panics — terminating the worker thread or, on auth, the entire process.

## Recommendation
Use `.unwrap_or_default()` (zero duration) or propagate as a typed error. For HMAC timestamps, falling back to zero is preferable to panic.

## References
- Related: SEC-083.
