# SEC-083: Weak HMAC Authentication - Replay Attack Vulnerability

## Status
- **Severity**: HIGH
- **Category**: Authentication
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Lines**: 505-553

## Description
The inter-node authentication uses a simple random challenge with HMAC-SHA256 without sequence numbers or timestamps, making it vulnerable to replay attacks.

## Exploit Scenario
Attacker captures challenge and response, then replays to authenticate as valid node.

## Recommendation
Add timestamps and nonces to prevent replay attacks.

## References
- Related: SEC-002 (predictable session ids)