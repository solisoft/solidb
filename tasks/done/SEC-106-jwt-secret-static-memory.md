# SEC-106: JWT Secret in Static Memory Without Protection

## Status
- **Severity**: HIGH
- **Category**: Secret Management
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 114-142

## Description
JWT_SECRET is stored in a `Lazy<String>` static without memory protection. Memory dumping attacks could extract the signing key.

## Exploit Scenario
Process memory dump reveals JWT signing key, enabling attacker to forge tokens.

## Recommendation
Consider using memory-protected secret storage in production.

## References
- Related: SEC-054 (jwt min secret weak), SEC-029 (jwt decode shape confusion)