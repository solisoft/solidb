# SEC-126: RBAC checks not enforced on privileged endpoints

## Status
- **Severity**: HIGH
- **Category**: Authorization
- **Project**: soli/db
- **File**: `src/server/handlers/databases.rs`, `src/server/handlers/auth.rs`, `src/server/script_handlers.rs`, `src/server/cluster_handlers.rs`, others
- **Lines**: handlers/databases.rs:90-110 (delete_database); handlers/auth.rs:140, 215, 247 (API key CRUD); cluster_handlers.rs:514-561 (remove_node, rebalance); script_handlers.rs (script/service/trigger CRUD); transaction_handlers.rs

## Description
Only `role_handlers.rs` calls `AuthorizationService::check_permission`. Every other privileged handler accepts any authenticated principal regardless of role. A `viewer` API key (with global_read only) can call `DELETE /_api/database/{db}`, create new admin API keys, install service scripts, or invoke `cluster_remove_node` / `cluster_rebalance`.

## Exploit Scenario
A read-only API key reaches `POST /_api/auth/api-keys` with `{role: "admin"}` and receives a new admin key — full privilege escalation, no special trick needed once SEC-124 is also in play (and even without it, since the handlers don't check roles at all).

## Recommendation
Add `AuthorizationService::check_permission(&claims, &state, action, scope)` at the top of every mutating handler. Particularly:
- DB/collection lifecycle: `Admin` on the database.
- API key CRUD, role/user CRUD: `Admin` global.
- Cluster ops: `Admin` global.
- Script/service/trigger CRUD: `Write` on target database (or `Admin`).

## References
- Depends on SEC-124 to make role checks meaningful.
