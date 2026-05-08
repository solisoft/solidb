# SEC-145: `change_password_handler` accepts arbitrarily weak passwords

## Status
- **Severity**: MEDIUM
- **Category**: Authentication / Input Validation
- **Project**: soli/db
- **File**: `src/server/handlers/auth.rs`
- **Lines**: 70-137

## Description
The change-password endpoint stores any string supplied as the new password — no length, complexity, or dictionary check. An admin compromise (or, currently, any logged-in user thanks to SEC-124) can demote the account to a 1-character password and propagate it through the replication log.

## Recommendation
- Enforce a minimum length (e.g., 12 characters).
- Reject obvious dictionary passwords (e.g., a small bundled blocklist).
- Optionally enforce zxcvbn-style scoring for higher-tier roles.

## References
- Related: SEC-091, SEC-106.
