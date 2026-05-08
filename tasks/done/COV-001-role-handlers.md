# COV-001: Cover `server/role_handlers.rs` (0% → ≥70%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/role_handlers.rs`
- **Current coverage**: 0% (641 lines uncovered)

## Description
RBAC role-administration endpoints (create/list/update/delete roles, assign/revoke role to user, list user roles, etc.) are not exercised by any test. `tests/rbac_admin_endpoints_tests.rs` only checks that admin-only endpoints reject viewers — it never hits the role handlers themselves.

## Recommendation
Add `tests/role_handlers_tests.rs` using the existing axum-oneshot pattern (see `tests/handlers_tests.rs`). `create_test_app` must call `engine.initialize()` so the `_system._roles` collection exists.

Endpoints to exercise (drive routes via `/_api/role*` URLs — see `src/server/routes.rs` for exact paths):
- POST create role (happy path + duplicate name)
- GET list roles (empty + populated)
- GET get role by name (found + not-found)
- PUT update role permissions (valid + invalid permission spec)
- DELETE role (existing + not-found, plus rejection if assigned to a user)
- POST assign role to user / revoke role from user
- GET list a user's roles
- AuthZ: each endpoint with viewer JWT → 403, missing JWT → 401

## Goal
Raise `src/server/role_handlers.rs` to ≥70% line coverage.

## References
- Pattern: `tests/handlers_tests.rs`, `tests/rbac_admin_endpoints_tests.rs`
- Coverage tool: `cargo llvm-cov --release --workspace --summary-only`
