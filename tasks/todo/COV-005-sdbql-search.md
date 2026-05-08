# COV-005: Cover `sdbql/executor/search.rs` (0% → ≥60%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/sdbql/executor/search.rs`
- **Current coverage**: 0% (476 lines uncovered)

## Description
The full-text search executor module is not exercised by any test. SDBQL search functions (`FULLTEXT(...)`, `MATCH(...)`, fuzzy/phrase queries) flow through this code, but there is no targeted test file for it.

## Recommendation
Add `tests/sdbql_search_tests.rs`. Build a small in-memory dataset via `StorageEngine::new` (no router needed), create a fulltext index, and exercise queries via the public SDBQL entry point used by other `sdbql_*_tests.rs` files. See `tests/sdbql_fuzzy_tests.rs` and `tests/sdbql_function_tests.rs` for the established setup.

Cases to exercise:
- Single-term match, multi-term AND/OR
- Phrase queries (`"exact phrase"`)
- Fuzzy / edit-distance matching
- Stop-word handling
- Score ordering (higher relevance first)
- Empty / non-existent index → graceful error
- Combination with `FILTER` / `SORT` / `LIMIT` clauses

## Goal
Raise `src/sdbql/executor/search.rs` to ≥60% line coverage.

## References
- Pattern: `tests/sdbql_fuzzy_tests.rs`, `tests/sdbql_function_tests.rs`
