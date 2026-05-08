# COV-002: Cover `server/handlers/blobs.rs` (0% → ≥60%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/handlers/blobs.rs`
- **Current coverage**: 0% (291 lines uncovered)

## Description
Public blob CRUD endpoints (upload, download, list, delete, metadata) have no test coverage. `tests/blob_distribution_tests.rs` only exercises the `/_internal/blob/*` cluster-replication routes, not the user-facing handlers.

## Recommendation
Add `tests/blob_handlers_tests.rs` using the axum-oneshot pattern. `create_test_app` must call `engine.initialize()` (see existing pattern after fix in `tests/blob_distribution_tests.rs`).

Endpoints to exercise:
- POST upload blob (single-shot + multipart) — happy path + missing content-type + payload too large
- GET download blob — found + not-found + 404 on non-blob collection
- GET blob metadata
- DELETE blob — existing + not-found + idempotency
- GET list blobs in collection
- AuthZ: each endpoint without JWT → 401, with insufficient role → 403
- Filename sanitization: SEC-166 already added CRLF stripping for `Content-Disposition` — assert it via a key containing `\r\n`.

## Goal
Raise `src/server/handlers/blobs.rs` to ≥60% line coverage.

## References
- Pattern: `tests/blob_distribution_tests.rs`, `tests/handlers_tests.rs`
- Related: SEC-166 (CRLF in blob filenames)
