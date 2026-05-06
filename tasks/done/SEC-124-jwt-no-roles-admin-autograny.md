# SEC-124: Login JWTs lack roles, triggering global-admin auto-grant

## Status
- **Severity**: CRITICAL
- **Category**: Privilege Escalation
- **Project**: soli/db
- **File**: `src/server/handlers/auth.rs`, `src/server/authorization.rs`, `src/server/auth.rs`
- **Lines**: handlers/auth.rs:351; authorization.rs:280-287; auth.rs:765-775

## Description
`login_handler` calls `AuthService::create_jwt(&user.username)` without populating roles, so the issued JWT has `roles: None`. `AuthorizationService::get_effective_permissions` then sees an empty role set and **inserts `Permission::global_admin()`** "for backward compatibility".

The same outcome occurs for Basic auth via `get_user_roles`, which auto-grants admin when the `_admins` collection has a single document.

Net effect: every successfully authenticating user — including a deliberately demoted viewer — receives a 24-hour admin JWT.

## Exploit Scenario
1. Admin creates a `viewer` API key for a user.
2. User logs in with their password.
3. The returned JWT carries `roles: None`.
4. The user reaches `DELETE /_api/database/{db}` — the authorization layer sees no roles → inserts `global_admin` → request succeeds.

## Recommendation
- In `login_handler`, populate roles via `create_jwt_with_roles(username, get_user_roles(...), scoped_databases)`.
- In `get_effective_permissions`, treat empty/missing roles as **no permissions**, not admin.
- Remove the "single admin auto-grant" in `get_user_roles` once a deliberate first-run setup flow exists.

## References
- Related: SEC-091, SEC-106.
