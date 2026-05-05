# SEC-100: No Authorization Checks on Internal Cluster Endpoints

## Status
- **Severity**: HIGH
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/server/handlers/cluster.rs`
- **Lines**: 459-497, 510-600

## Description
Internal cluster management endpoints (`cluster_cleanup`, `cluster_reshard`) only check for cluster secret but don't verify requesting node is a legitimate cluster member.

## Exploit Scenario
Attacker with knowledge of cluster secret can trigger cleanup or resharding operations.

## Recommendation
Add verification that requesting node is actually a cluster member and request is part of legitimate operation.

## References
- Related: SEC-081 (auth bypass no keyfile), SEC-084 (cluster secret plaintext)