# SEC-131: SDBQL range expansion eagerly allocates

## Status
- **Severity**: HIGH
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/sdbql/executor/expression.rs`
- **Lines**: 306-310

## Description
Range expressions evaluate via `(start..=end).collect::<Vec<Value>>()`, materializing the entire range in memory before iteration. The SDBQL query timeout fires from `tokio::time::timeout`, which cannot interrupt a synchronous allocation running inside `spawn_blocking`.

## Exploit Scenario
```sdbql
FOR i IN 0..1000000000 RETURN 1
```
Allocates ~16 GB before any timeout can fire — process is killed by the OOM-killer.

## Recommendation
- Cap range size (reject `end - start > 10_000_000`, configurable).
- Stream via an iterator-backed source so the executor and FOR loop can short-circuit on timeout/limit.

## References
- Related: SEC-094.
