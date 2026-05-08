# COV-008: Cover `transaction/distributed.rs` (0% → ≥50%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/transaction/distributed.rs`
- **Current coverage**: 0% (283 lines uncovered)

## Description
Distributed (two-phase-commit) transaction logic is uncovered. `tests/transaction_handlers_tests.rs` only exercises the local single-node transaction manager.

## Recommendation
Two complementary layers:

1. **Unit tests**: extract pure-logic helpers (state transitions, vote tallying, prepare/commit/abort decisioning, timeout calculation) into testable functions and assert them directly without networking.

2. **Integration tests**: spin up two `StorageEngine`s in the same process, wire them via the in-process trait used by sharding tests, and drive a 2PC across them. Cases:
   - Happy path: prepare on both → commit on both.
   - One participant votes abort → coordinator aborts on both.
   - One participant times out during prepare → coordinator aborts.
   - Coordinator crash after prepare (recovery: participants honor commit on replay).
   - Idempotency: replayed commit/abort is a no-op.

## Goal
Raise `src/transaction/distributed.rs` to ≥50% line coverage.

## References
- Pattern: `tests/transaction_handlers_tests.rs`
- Local manager: `src/transaction/manager.rs` (currently 80%)
