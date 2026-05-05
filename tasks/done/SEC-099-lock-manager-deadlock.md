# SEC-099: Lock Manager Potential Deadlock Scenario

## Status
- **Severity**: MEDIUM
- **Category**: Race Condition
- **Project**: soli/db
- **File**: `src/transaction/lock_manager.rs`
- **Lines**: 44-85, 87-144, 146-182

## Description
The lock manager acquires shared locks by first checking exclusive locks, then acquiring shared locks. The upgrade function removes from shared before adding to exclusive without atomic upgrade.

## Exploit Scenario
Transaction could hold a shared lock while another transaction tries to upgrade, leaving documents in inconsistent state.

## Recommendation
Consider using a single lock acquisition point with proper lock escalation semantics.

## References
- Related: SEC-096 (transaction toctou)