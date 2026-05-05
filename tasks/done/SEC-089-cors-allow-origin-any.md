# SEC-089: CORS Allow-Origin Any

## Status
- **Severity**: CRITICAL
- **Category**: Configuration
- **Project**: soli/db
- **File**: `src/server/routes.rs`
- **Lines**: 1039-1050

## Description
The CORS layer is configured with `allow_origin(Any)`, allowing cross-origin requests from any domain.

## Exploit Scenario
An attacker hosting a malicious page could steal auth tokens or perform actions on behalf of admin users.

## Recommendation
Restrict CORS to specific trusted origins in production.

## References
- Related: SEC-028 (cookie secure depends trust proxy)