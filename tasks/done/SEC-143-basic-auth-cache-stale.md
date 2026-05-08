# SEC-143: Basic-auth cache not invalidated on password change / user delete

## Status
- **Severity**: MEDIUM
- **Category**: Authentication
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 885-921 (Basic-auth cache lookup/insertion)

## Description
Successful Basic auth is cached for 60 s keyed on `username:SipHash(credentials)`. After `change_password_handler` updates the hash, or `delete_user` removes the user, the old credentials still authenticate for up to 60 s.

## Exploit Scenario
- A password is briefly leaked, then rotated. The attacker gets a 60 s grace window past the rotation.
- An admin compromise is detected and the account deleted; the deleted account remains usable for 60 s.

## Recommendation
- On password change / user delete / role revoke, purge entries with the matching `username:` prefix from `BASIC_AUTH_CACHE`.
- Same purge for `permission_cache` if present.

## References
- Related: SEC-091.
