# COV-003: Cover `server/handlers/sync.rs` (0% → ≥60%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/handlers/sync.rs`
- **Current coverage**: 0% (452 lines uncovered)

## Description
Offline-first sync endpoints (`/_api/sync/session`, `/_api/sync/pull`, `/_api/sync/push`, `/_api/sync/ack`, `/_api/sync/conflicts`, `/_api/sync/resolve`) are unexercised. SEC-154 fixed framing inconsistencies in the sync protocol but the route handlers themselves have no integration test.

## Recommendation
Add `tests/sync_handlers_tests.rs`. `create_test_app` must:
- Call `engine.initialize()`.
- Configure a cluster keyfile via `StorageEngine::with_cluster_config` so `SyncSession::verify_session_id` (which uses the cluster secret) doesn't no-op silently.

Endpoints to exercise:
- POST `/_api/sync/session` — register a sync session, capture returned session id.
- POST `/_api/sync/pull` — empty change set (cold start) + after some inserts.
- POST `/_api/sync/push` — valid batch + invalid framing → 400 (regression for SEC-154).
- POST `/_api/sync/ack` — happy path + bad session id → 401/403.
- GET `/_api/sync/conflicts` — empty + with simulated conflicts.
- POST `/_api/sync/resolve` — accept-local / accept-remote / merge.
- AuthZ: each endpoint without JWT → 401.

## Goal
Raise `src/server/handlers/sync.rs` to ≥60% line coverage.

## References
- Pattern: `tests/handlers_tests.rs`, `tests/sync_protocol_tests.rs`
- Related: SEC-154
