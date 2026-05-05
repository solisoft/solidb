# SEC-080: No TLS/SSL Encryption for Inter-Node Communication

## Status
- **Severity**: CRITICAL
- **Category**: Transport Security
- **Project**: soli/db
- **Files**: `src/sync/transport.rs`, `src/cluster/transport.rs`

## Description
All inter-node replication traffic uses plaintext TCP connections. No TLS encryption is used for the binary sync protocol or cluster management messages.

## Exploit Scenario
An attacker with network access between cluster nodes can eavesdrop on all replication traffic to capture document content or inject fake data.

## Recommendation
Enable mutual TLS (mTLS) for both TCP sync protocol and HTTP API.

## References
- Related: SEC-027 (db config forces http), SEC-042 (tls min version)