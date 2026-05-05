# SEC-115: No Database Name Validation

## Status
- **Severity**: MEDIUM
- **Category**: Input Validation
- **Project**: soli/db
- **Files**: Throughout handlers (env_handlers.rs, script_handlers.rs, role_handlers.rs)

## Description
Database names from URLs are used directly without validation.

## Exploit Scenario
Malformed or malicious database names cause unexpected behavior.

## Recommendation
Validate database names against expected patterns.

## References
- Related: SEC-103 (collection name validation)