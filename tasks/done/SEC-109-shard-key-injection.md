# SEC-109: Shard Key Injection in Routing

## Status
- **Severity**: MEDIUM
- **Category**: Injection
- **Project**: soli/db
- **File**: `src/sharding/router.rs`, `src/sync/protocol.rs`
- **Lines**: 11-19, 418-426

## Description
The shard routing uses a simple hash of document key (`seahash::hash(key.as_bytes())`). Attacker can craft document keys that hash to specific shard IDs.

## Exploit Scenario
Attacker creates documents with keys crafted to hash all data to a single shard, overwhelming specific nodes.

## Recommendation
Add validation or salting to prevent shard key manipulation.

## References
- Related: SEC-004c (qb chain field injection)