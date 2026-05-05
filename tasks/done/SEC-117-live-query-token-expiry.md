# SEC-117: Live Query Token 2-Second Expiry Causes Auth Failures

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 616-637

## Description
Live query token expires in 2 seconds. Legitimate clients may not have sufficient time to establish WebSocket connection.

## Exploit Scenario
Repeated auth failures causing service disruption.

## Recommendation
Consider increasing expiration to 30 seconds while maintaining security.

## References
- Related: SEC-042 (tls min version)