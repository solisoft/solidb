# SEC-137: `JoinRequest` accepts unverified node identity

## Status
- **Severity**: HIGH
- **Category**: Authentication / Cluster Integrity
- **Project**: soli/db
- **File**: `src/cluster/manager.rs`
- **Lines**: 228-242

## Description
`JoinRequest` is accepted with whatever `node.id` and `node.address` the caller asserts. Any TCP-reachable party can claim to be node `victim` at address `victim.example.com`, get registered into cluster metadata, and start receiving shard ops/heartbeats.

## Exploit Scenario
1. Attacker connects to a cluster member.
2. Sends `JoinRequest { node: { id: "node-3", address: "evil.example.com" } }`.
3. Cluster updates membership; subsequent shard ops route through the attacker.
4. Combined with HLC manipulation (SEC-138), causes split-brain.

## Recommendation
- Tie cluster transport to the keyfile-authenticated identity (per SEC-080 architecture).
- Require `node.id` to match the authenticated peer principal.
- Optionally require operator approval for new node IDs.

## References
- Related: SEC-080, SEC-108, SEC-110.
