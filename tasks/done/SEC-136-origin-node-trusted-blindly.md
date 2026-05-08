# SEC-136: `origin_node` / `origin_sequence` trusted from the wire

## Status
- **Severity**: HIGH
- **Category**: Replication Integrity
- **Project**: soli/db
- **File**: `src/sync/worker.rs`, `src/cluster/manager.rs`
- **Lines**: worker.rs:582, 919; manager.rs:312-326

## Description
Replication entries arrive with `origin_node` and `origin_sequence` fields that the worker uses for de-duplication. The receiving node never checks that `origin_node` matches the authenticated peer identity — a connected peer can emit entries claiming to originate from any node, advancing that node's high-watermark.

## Exploit Scenario
Peer A is authenticated via the cluster keyfile but emits `SyncEntry { origin_node: "B", origin_sequence: N+1, ... }`. Other nodes record N+1 as the high watermark for B. When B's real entry N+1 arrives, it is dropped as duplicate — silent data loss / hiding.

## Recommendation
- Bind authenticated peer identity to the connection.
- Reject `SyncEntry` whose `origin_node` ≠ connection peer ID, unless the message is a fan-out within the origin's HLC chain (validate the chain).
- Log + ban-list peers that violate this invariant.

## References
- Related: SEC-080, SEC-110, SEC-135.
