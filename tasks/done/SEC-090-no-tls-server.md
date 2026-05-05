# SEC-090: No TLS for Server (Binds to 0.0.0.0 Without Encryption)

## Status
- **Severity**: CRITICAL
- **Category**: Transport Security
- **Project**: soli/db
- **File**: `src/main.rs`
- **Lines**: 533, 676

## Description
Server binds to `0.0.0.0:6745` without any TLS encryption. All data including passwords and JWT tokens transmitted in plaintext.

## Exploit Scenario
Network eavesdropping (MITM attack) can intercept all credentials and data in transit.

## Recommendation
Enable TLS support or document that HTTP should be behind a TLS terminator.

## References
- Related: SEC-027 (db config forces http), SEC-080 (no TLS inter-node)