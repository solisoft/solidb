# SEC-141: `TransactionId` collisions under high concurrency

## Status
- **Severity**: HIGH
- **Category**: Data Integrity
- **Project**: soli/db
- **File**: `src/transaction/mod.rs`, `src/transaction/manager.rs`
- **Lines**: mod.rs:17-24 (`TransactionId::new`); manager.rs:54 (`active_transactions` insert)

## Description
`TransactionId::new()` truncates `SystemTime::now().duration_since(UNIX_EPOCH).as_nanos()` to `u64` and uses it both as the unique ID and the MVCC `read_timestamp`. Two concurrent BEGINs within the same nanosecond collide. The receiving `HashMap` then **silently overwrites** one transaction with the other.

## Exploit Scenario
Under burst load, two concurrent BEGINs receive the same ID. The second insert clobbers the first. Commit/rollback of the surviving ID releases the wrong locks; the lost transaction's WAL `BEGIN` record is orphaned, breaking recovery invariants.

## Recommendation
- Use a monotonic atomic counter (`AtomicU64::fetch_add`) for the ID, seeded once at startup from system time.
- Keep `read_timestamp` separate (e.g., HLC tick) for MVCC.

## References
- Related: SEC-096, SEC-099.
