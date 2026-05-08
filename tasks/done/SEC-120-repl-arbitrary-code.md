# SEC-120: REPL Endpoint - Arbitrary Lua Code Execution

## Status
- **Severity**: CRITICAL
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/server/script_handlers.rs`
- **Lines**: 469-545

## Description
The REPL endpoint executes arbitrary Lua code provided by authenticated users with full database API access via `ScriptEngine`.

## Exploit Scenario
Compromised or malicious authenticated user executes harmful Lua code to read/modify/delete all data or execute system commands.

## Recommendation
Restrict REPL endpoint access, consider requiring separate elevated permissions.

## References
- Related: SEC-091 (permissive auth anonymous), SEC-077 (ssrf solidb fetch)