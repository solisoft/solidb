# SEC-079: Directory Traversal in solidb.upload()

## Status
- **Severity**: HIGH
- **Category**: Path Traversal
- **Project**: soli/db
- **File**: `src/scripting/file_handling.rs`
- **Lines**: 209-215

## Description
The upload function sanitizes directory paths with basic string replacement (`replace("..", "")`) which can be bypassed with encoding tricks like `....//` or `/../`.

## Exploit Scenario
```lua
solidb.upload(data, { filename = "test.txt", directory = "....//....//etc/" })
solidb.upload(data, { filename = "shell.jsp", directory = "%2e%2e%2f%2e%2e%2f" })
```

## Recommendation
Use canonical path resolution and enforce directory allowlisting.

## References
- Related: SEC-078, SEC-011 (zip slip)