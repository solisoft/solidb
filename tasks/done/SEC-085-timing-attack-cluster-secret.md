# SEC-085: Timing Attack in Cluster Secret Comparison

## Status
- **Severity**: HIGH
- **Category**: Cryptography
- **Project**: soli/db
- **Files**: `src/server/handlers/cluster.rs`, `src/server/cluster_handlers.rs`
- **Lines**: 471, 522, 577, 628

## Description
Uses regular `!=` operator instead of `constant_time_eq` for comparing cluster secrets.

## Exploit Scenario
Attacker can determine the correct cluster secret byte-by-byte using timing analysis.

## Recommendation
Replace `!=` with `constant_time_eq` in all cluster handlers.

## References
- Related: SEC-029 (jwt decode shape confusion), SEC-053 (session id not validated)