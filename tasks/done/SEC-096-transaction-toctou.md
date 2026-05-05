# SEC-096: Transaction Validation TOCTOU Race Condition

## Status
- **Severity**: MEDIUM
- **Category**: Race Condition
- **Project**: soli/db
- **File**: `src/transaction/manager.rs`
- **Lines**: 79-151

## Description
The `validate` function collects errors without holding the transaction lock, then adds them while holding the lock. Transaction state could change between phases.

## Exploit Scenario
Between releasing read lock and acquiring write lock, another thread could modify the transaction, potentially bypassing validation.

## Recommendation
Keep validation under a single lock acquisition or use atomic compare-and-swap patterns.

## References
- Related: SEC-039 (uniqueness toctou), SEC-008 (deadlock scenarios)