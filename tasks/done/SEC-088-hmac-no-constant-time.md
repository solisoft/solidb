# SEC-088: HMAC Comparison Not Constant-Time

## Status
- **Severity**: HIGH
- **Category**: Cryptography
- **Project**: soli/db
- **File**: `src/sync/transport.rs`
- **Line**: 541

## Description
HMAC comparison uses regular `==` operator instead of constant-time comparison.

## Exploit Scenario
Timing attacks could reveal the keyfile content used for HMAC authentication.

## Recommendation
Use `subtle::ConstantTimeEq` for HMAC comparison.

## References
- Related: SEC-085 (timing attack cluster secret)