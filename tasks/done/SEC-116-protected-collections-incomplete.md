# SEC-116: Protected Collections List Incomplete

## Status
- **Severity**: MEDIUM
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/server/handlers/system.rs`
- **Lines**: 11-18

## Description
Only `_admins` and `_api_keys` are protected. Collections like `_roles`, `_user_roles`, `_scripts`, `_services` are not explicitly protected.

## Exploit Scenario
Access to security-sensitive collections without proper authorization.

## Recommendation
Expand protected collections list to include all security-sensitive collections.

## References
- Related: SEC-103 (collection name validation), SEC-091 (permissive auth)