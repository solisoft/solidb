# SEC-149: `explain_query` has no timeout / `spawn_blocking`

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/server/handlers/query.rs`
- **Lines**: 683-711

## Description
Unlike `execute_query`, `explain_query` does not wrap the planner call in `tokio::time::timeout` or `spawn_blocking`. A query designed to do deep planner analysis can pin an async runtime thread.

## Recommendation
Mirror the pattern from `execute_query`: run the planner inside `spawn_blocking`, wrapped in a `tokio::time::timeout(QUERY_TIMEOUT_SECS, ...)`.

## References
- Related: SEC-094.
