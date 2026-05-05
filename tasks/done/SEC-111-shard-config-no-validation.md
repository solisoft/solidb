# SEC-111: No Shard Configuration Validation

## Status
- **Severity**: MEDIUM
- **Category**: Validation
- **Project**: soli/db
- **Files**: `src/sharding/coordinator.rs`, `src/cluster/manager.rs`
- **Lines**: 481-505

## Description
When receiving shard configurations via replication, there is no validation that configuration values are within acceptable bounds.

## Exploit Scenario
Malicious `num_shards: u16::MAX` (65535) could cause infinite loops; `replication_factor: u16::MAX` tries to create excessive replicas.

## Recommendation
Add bounds checking on num_shards, replication_factor.

## References
- Related: SEC-062 (bind values untyped), SEC-109 (shard key injection)