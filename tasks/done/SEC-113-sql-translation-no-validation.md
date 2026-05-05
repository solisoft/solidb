# SEC-113: SQL Query Translation No Input Validation

## Status
- **Severity**: MEDIUM
- **Category**: Injection
- **Project**: soli/db
- **File**: `src/server/sql_handlers.rs`
- **Lines**: 38-106

## Description
SQL queries are passed directly to `translate_sql_to_sdbql` without validation before translation.

## Exploit Scenario
Translation vulnerabilities could be exploited via specially crafted SQL.

## Recommendation
Add input validation before SQL-to-SDBQL translation.

## References
- Related: SEC-035 (transaction sdbql injection), SEC-104 (template string injection)