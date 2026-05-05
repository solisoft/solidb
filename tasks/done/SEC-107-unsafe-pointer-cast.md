# SEC-107: Unsafe Pointer Cast in Collection Creation

## Status
- **Severity**: MEDIUM
- **Category**: Memory Safety
- **Project**: soli/db
- **File**: `src/storage/engine.rs`
- **Lines**: 409-414

## Description
Uses unsafe pointer cast for RocksDB column family creation.

## Exploit Scenario
Undefined behavior if pointer handling is incorrect.

## Recommendation
Consider safer abstractions or thorough testing of the unsafe code path.

## References
- Related: SEC-074 (system class arc non send sync) in lang