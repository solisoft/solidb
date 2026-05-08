# SEC-147: Cursor `batch_size` is unbounded

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/server/handlers/query.rs`
- **Lines**: 34-46, 541, 665

## Description
The `batch_size` field of `ExecuteQueryRequest` is deserialized as a plain `usize` with no upper cap. With `batch_size = usize::MAX`, `store_and_get_first_batch` eagerly drains the iterator into `first_batch: Vec<Value>` and skips cursor storage entirely, defeating pagination's memory amortization.

## Recommendation
Clamp on the way in: `let batch_size = req.batch_size.unwrap_or(DEFAULT).min(MAX_BATCH);` with `MAX_BATCH` around 10 000. Document the default and ceiling.

## References
- Related: SEC-094.
