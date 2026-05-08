# SEC-098: Path Sanitization Only Replaces Characters (Incomplete)

## Status
- **Severity**: MEDIUM
- **Category**: Path Traversal
- **Project**: soli/db
- **File**: `src/server/script_handlers.rs`
- **Lines**: 375-379

## Description
The `sanitize_path_to_key` function only replaces `/`, `:`, and `*` with underscores but doesn't validate for path traversal sequences like `../`.

## Exploit Scenario
A path like `users/../../etc/passwd` sanitized to `users____etc_passwd` could still be exploited if used in file operations.

## Recommendation
Add validation to reject paths containing `..` or ensure sanitized output cannot be used for file system access.

## References
- Related: SEC-078, SEC-079