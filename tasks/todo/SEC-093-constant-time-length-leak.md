# SEC-093: Constant-Time Comparison Length Leak

## Status
- **Severity**: MEDIUM
- **Category**: Cryptography
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 759-764

## Description
The `constant_time_eq` function returns immediately when lengths differ, leaking the length of secrets via timing.

## Exploit Scenario
Attacker can determine the length of secrets by measuring response time.

## Recommendation
Use `subtle::ConstantTimeEq` from Rust's crypto libraries for proper constant-time comparison.

## References
- Related: SEC-085, SEC-088