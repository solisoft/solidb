# SEC-148: `LIMIT offset + count` overflow in executor

## Status
- **Severity**: MEDIUM
- **Category**: Logic / DoS
- **Project**: soli/db
- **File**: `src/sdbql/executor/execution/entry.rs`
- **Lines**: 121

## Description
`limit_offset + limit_count` can wrap silently in release builds when both come from large bind variables, producing an effective `Some(0)` and causing a full collection scan (or unexpected behavior).

## Recommendation
Use `checked_add`; on overflow, return a query error rather than silently scanning everything.

## References
- Related: SEC-094, SEC-118.
