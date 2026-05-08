# SEC-146: `X-Forwarded-For` trusted unconditionally for rate limiting

## Status
- **Severity**: MEDIUM
- **Category**: Rate Limiting Bypass
- **Project**: soli/db
- **File**: `src/server/handlers/auth.rs`
- **Lines**: 283-294

## Description
`login_handler` derives the rate-limit key from the `X-Forwarded-For` header without any proxy allowlist. Any direct caller can set `X-Forwarded-For: <random>` per request and reset the per-IP counter, defeating the 20-attempts/60-s login rate limit.

## Exploit Scenario
A brute-forcing client iterates through random `X-Forwarded-For` values, never tripping the 20-attempt threshold for any single key.

## Recommendation
- Only honor `X-Forwarded-For` when the connection arrives from a configured trusted-proxy CIDR (`SOLIDB_TRUSTED_PROXIES`).
- Otherwise, key the rate limit on the socket peer address.

## References
- Related: SEC-105, SEC-092.
