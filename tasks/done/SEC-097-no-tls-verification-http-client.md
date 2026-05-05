# SEC-097: No TLS Verification for HTTP Client

## Status
- **Severity**: HIGH
- **Category**: Transport Security
- **Project**: soli/db
- **File**: `src/storage/http_client.rs`

## Description
The HTTP client used for inter-node HTTP communication does not verify TLS certificates.

## Exploit Scenario
Man-in-the-middle attack during shard healing/copying operations allows attacker to intercept and modify data.

## Recommendation
Configure the HTTP client to verify TLS certificates.

## References
- Related: SEC-080 (no TLS inter-node), SEC-042a (tls min version cli)