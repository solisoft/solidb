# SEC-094: Unbounded Query Resource Exhaustion

## Status
- **Severity**: HIGH
- **Category**: DoS
- **Project**: soli/db
- **File**: `src/server/handlers/query.rs`
- **Lines**: 20-21

## Description
While `QUERY_TIMEOUT_SECS` constant exists (30 seconds), it is not actively enforced during query execution.

## Exploit Scenario
```sql
FOR i IN 1..1000000000000 INSERT {} INTO huge_collection
```
Would attempt to create a trillion documents.

## Recommendation
Implement timeout enforcement using `tokio::time::timeout`.

## References
- Related: SEC-020 (unbounded thread fanout), SEC-068 (repl session unbounded)