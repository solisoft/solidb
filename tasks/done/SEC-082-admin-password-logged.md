# SEC-082: Admin Password Logged in Plaintext

## Status
- **Severity**: CRITICAL
- **Category**: Information Disclosure
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 309-333

## Description
When admin account is created with random password, the plaintext password is printed to stderr via `tracing::warn!()`.

## Exploit Scenario
Anyone with access to server logs or terminal output can retrieve the admin password.

## Recommendation
Remove password logging entirely or ensure it only appears in secure dev mode.

## References
- Related: SEC-060 (http log userinfo secrets)