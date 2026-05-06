# SEC-140: `TransactionManager::cleanup_expired` is never called

## Status
- **Severity**: HIGH
- **Category**: Denial of Service / Resource Leak
- **Project**: soli/db
- **File**: `src/transaction/manager.rs`
- **Lines**: 240-266 (`cleanup_expired`)

## Description
No caller invokes `cleanup_expired()` anywhere in the binary. The default 300 s transaction timeout is documented but never enforced. Long-running transactions accumulate row locks via the lock manager and a `Transaction` Arc forever.

## Exploit Scenario
An authenticated user repeatedly opens transactions without committing or rolling back. Each holds row locks indefinitely, blocking writers on the same keys. Memory grows unbounded.

## Recommendation
- Spawn a tokio interval task at server boot calling `tx_manager.cleanup_expired()` every 30 s.
- Bound `active_transactions.len()` per principal (e.g., 16 concurrent).
- Emit metrics for active transactions and aged-out expirations.

## References
- Related: SEC-096, SEC-099.
