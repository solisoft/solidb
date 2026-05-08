# SEC-101: Error Messages Expose Internal Details

## Status
- **Severity**: MEDIUM
- **Category**: Information Disclosure
- **Project**: soli/db
- **File**: `src/error.rs`
- **Lines**: 100-127

## Description
Error responses expose internal details like file paths, internal IPs, or implementation details via `DbError::InternalError`.

## Exploit Scenario
Helps attackers understand system internals and craft targeted attacks.

## Recommendation
Sanitize error messages before returning to clients.

## References
- Related: SEC-009 (unauth dev source disclosure), SEC-059 (aws creds in error strings)