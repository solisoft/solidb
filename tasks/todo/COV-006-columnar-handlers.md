# COV-006: Cover `server/columnar_handlers.rs` (0% → ≥60%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/columnar_handlers.rs`
- **Current coverage**: 0% (284 lines uncovered)

## Description
Columnar (Parquet/Arrow) export/import endpoints have no test coverage. Pure-storage tests exist (`tests/columnar_index_tests.rs`, `tests/columnar_tests.rs`) but the HTTP handler layer is unexercised.

## Recommendation
Add `tests/columnar_handlers_tests.rs` using the axum-oneshot pattern (`engine.initialize()` required).

Endpoints to exercise (consult `src/server/routes.rs` for the exact paths):
- POST export collection to columnar — happy path round-trip (export then re-import).
- GET column statistics endpoint.
- POST query against columnar storage.
- Error cases: non-existent collection → 404, invalid format → 400.
- AuthZ: missing JWT → 401, viewer JWT on write endpoint → 403.

## Goal
Raise `src/server/columnar_handlers.rs` to ≥60% line coverage.

## References
- Pattern: `tests/handlers_tests.rs`
- Storage layer tests: `tests/columnar_tests.rs`
