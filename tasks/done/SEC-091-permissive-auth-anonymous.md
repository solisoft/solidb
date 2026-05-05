# SEC-091: Permissive Auth Middleware Allows Anonymous Access

## Status
- **Severity**: HIGH
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 926-1021

## Description
The `permissive_auth_middleware` allows requests to proceed without authentication when no auth headers are present.

## Exploit Scenario
Service scripts that rely on this middleware may be accessible to unauthenticated users.

## Recommendation
Add explicit authentication checks to service scripts.

## References
- Related: SEC-075 (missing auth query endpoint)