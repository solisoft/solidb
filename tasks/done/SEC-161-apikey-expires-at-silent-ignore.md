# SEC-161: `ApiKey.expires_at` parse failure silently treated as never-expiring

## Status
- **Severity**: LOW
- **Category**: Authorization / Configuration
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 717-723

## Description
The `expires_at` field is parsed via `if let Ok(_) = OffsetDateTime::parse(...)`. When parsing fails (operator typo, schema drift), the inner block is simply skipped and the key is treated as never-expiring.

## Recommendation
Treat unparseable `expires_at` as expired (fail closed) and log the corruption. Add a startup linter that scans `_api_keys` for malformed dates and refuses to serve them.

## References
- Related: SEC-106.
