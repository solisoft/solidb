# SEC-110: Gossip Protocol Security Issues

## Status
- **Severity**: MEDIUM
- **Category**: Protocol Security
- **Project**: soli/db
- **Files**: `src/cluster/manager.rs`, `src/cluster/health.rs`

## Description
The cluster uses simple heartbeat-based gossip without authentication or integrity verification. Fake heartbeats can be injected.

## Exploit Scenario
Mark healthy nodes as dead (unnecessary failover) or dead nodes as healthy (preventing proper failover).

## Recommendation
Add authentication and integrity verification to gossip messages.

## References
- Related: SEC-080 (no TLS inter-node), SEC-108 (trust on first connect)