# SEC-155: Scatter-gather coordinator amplifies user requests across the cluster

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service / Authorization
- **Project**: soli/db
- **File**: `src/sharding/coordinator.rs`
- **Lines**: ~1885+ (`upsert_batch_to_shards`), 1182-1212 (`_copy_shard`)

## Description
A single client write fans out to N shards × R replicas via internal HTTP using `X-Shard-Direct`. The coordinator never re-checks whether the originating user has rights on the *target physical* shard collection — only on the logical one. There is no per-request fan-out budget.

## Exploit Scenario
- An authenticated user with read-only access on `users` triggers writes that fan out across the cluster, generating O(N×R) outbound HTTP requests.
- Combined with SEC-109 (still-incomplete shard-key validation), the user may even land writes on shards they shouldn't.

## Recommendation
- Apply a per-request fan-out budget (configurable max).
- On the receiving node, recheck authorization against the **logical** collection name, not just the cluster secret.
- Surface fan-out metrics for capacity planning.

## References
- Related: SEC-100, SEC-109, SEC-111.
