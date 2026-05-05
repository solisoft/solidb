# SEC-092: No Rate Limiting on Admin Endpoints

## Status
- **Severity**: MEDIUM
- **Category**: Rate Limiting
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 51-77

## Description
Rate limiting only applies to login attempts. Other sensitive endpoints like password change, API key management have no rate limiting.

## Exploit Scenario
Brute force attacks against sensitive admin endpoints.

## Recommendation
Add rate limiting to all sensitive endpoints.

## References
- Related: SEC-030 (rate limit xff spoof)