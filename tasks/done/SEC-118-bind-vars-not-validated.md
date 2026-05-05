# SEC-118: Bind Variables Not Validated

## Status
- **Severity**: MEDIUM
- **Category**: Input Validation
- **Project**: soli/db
- **File**: `src/sdbql/executor/expression.rs`
- **Lines**: 91-103

## Description
Bind variables (`@name`) passed in requests are not validated before use.

## Exploit Scenario
Malicious bind variable values could exploit logic vulnerabilities in query execution.

## Recommendation
Validate bind variable contents against expected types/schemas.

## References
- Related: SEC-062 (bind values untyped), SEC-113 (sql translation no validation)