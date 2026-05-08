# SEC-103: Collection Name Validation Missing

## Status
- **Severity**: HIGH
- **Category**: Access Control
- **Project**: soli/db
- **Files**: `src/sdbql/parser/clauses.rs`, `src/sdbql/executor/data_source.rs`
- **Lines**: 194-231, 259-296, 629-637

## Description
Collection names parsed from queries are not validated or sanitized. Allows access to internal system collections like `_roles`, `_admins`, `_user_roles`.

## Exploit Scenario
```sql
FOR doc IN _admins RETURN doc
```
Could potentially return all admin users including password hashes.

## Recommendation
Validate collection names don't start with `_` for non-admin users.

## References
- Related: SEC-003 (mass assignment), SEC-076 (sleep blind injection)