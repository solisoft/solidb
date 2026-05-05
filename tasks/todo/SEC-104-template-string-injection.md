# SEC-104: Template String Injection in SDBQL

## Status
- **Severity**: HIGH
- **Category**: Injection
- **Project**: soli/db
- **File**: `src/sdbql/lexer.rs`
- **Lines**: 253-311

## Description
Template strings (`$"..."` with `${expression}` interpolation) evaluate expressions directly. If user input flows into template strings without proper escaping, injection is possible.

## Exploit Scenario
User input containing `${恶意代码}` could be evaluated as expression.

## Recommendation
Validate and sanitize template string expressions before evaluation.

## References
- Related: SEC-076 (sleep blind injection), SEC-058 (erb no context escape)