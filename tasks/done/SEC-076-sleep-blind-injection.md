# SEC-076: Time-Based Blind Injection via SLEEP Function

## Status
- **Severity**: HIGH
- **Category**: Injection
- **Project**: soli/db
- **File**: `src/sdbql/executor/builtins/misc.rs`
- **Lines**: 66-73

## Description
The `SLEEP(ms)` function allows blocking execution for arbitrary durations. Combined with conditional execution, this enables time-based blind injection attacks.

## Exploit Scenario
```sql
FOR doc IN users FILTER SLEEP(doc.password == 'admin' ? 5000 : 0) RETURN doc
```

## Recommendation
Consider removing the SLEEP function entirely or restricting it to admin-only usage.

## References
- Related: SEC-035 (transaction sdbql injection)