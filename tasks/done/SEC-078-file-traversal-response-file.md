# SEC-078: Arbitrary File System Access via response.file()

## Status
- **Severity**: HIGH
- **Category**: Path Traversal
- **Project**: soli/db
- **File**: `src/scripting/http_helpers.rs`
- **Lines**: 171-198

## Description
The `response.file(path)` function accepts a file path parameter and uses `std::fs::metadata()` to check file existence without path sanitization. This allows directory traversal attacks.

## Exploit Scenario
```lua
local f = response.file("/etc/passwd")
local f = response.file("../../secrets/secrets.yaml")
```

## Recommendation
Implement strict path validation, restrict to a designated upload directory, and resolve paths before access.

## References
- Related: SEC-006, SEC-010 (existing file traversal issues)