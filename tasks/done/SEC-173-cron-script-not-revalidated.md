# SEC-173: Cron-spawned jobs inherit unvalidated `script_path`

## Status
- **Severity**: LOW
- **Category**: Injection (depends on SEC-125)
- **Project**: soli/db
- **File**: `src/queue/cron.rs`
- **Lines**: 96-112 (job spawn from cron entry)

## Description
When a cron entry fires, the spawned `Job` inherits the cron entry's `script_path` and `params` with no re-validation. If the cron was created with a malicious value (see SEC-125), the SDBQL injection persists until cron deletion.

## Recommendation
Validate `script_path` at cron CREATE/UPDATE time using the same allowlist regex applied at enqueue (SEC-125). Add a startup linter that rejects existing cron entries with invalid paths.

## References
- Depends on SEC-125.
