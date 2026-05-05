# SEC-108: Trust on First Connect - No Node Identity Verification

## Status
- **Severity**: HIGH
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/cluster/manager.rs`
- **Lines**: 219-224, 226-241

## Description
When a node joins the cluster, it accepts the join request without verifying the node's identity beyond its claimed address.

## Exploit Scenario
Attacker spins up malicious node, sends JoinRequest, receives full cluster topology and replication traffic.

## Recommendation
Implement proper node identity verification using certificates or shared secrets.

## References
- Related: SEC-081 (auth bypass no keyfile), SEC-080 (no TLS inter-node)