# SEC-084: Cluster Secret Transmitted in Plaintext Headers

## Status
- **Severity**: HIGH
- **Category**: Secret Management
- **Project**: soli/db
- **Files**: `src/cluster/websocket_client.rs`, `src/sharding/coordinator.rs`

## Description
The cluster secret is sent in the `X-Cluster-Secret` HTTP header without TLS encryption.

## Exploit Scenario
Network sniffing reveals the cluster authentication secret, enabling attacker to make direct API calls.

## Recommendation
Use TLS and consider using certificate-based node authentication instead of secrets in headers.

## References
- Related: SEC-080 (no TLS inter-node)