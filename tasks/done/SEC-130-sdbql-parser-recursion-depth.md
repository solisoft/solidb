# SEC-130: SDBQL parser has no recursion-depth limit

## Status
- **Severity**: HIGH
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/sdbql/parser/mod.rs`, `src/sdbql/parser/expressions/precedence.rs`, `src/sdbql/parser/expressions/primary.rs`
- **Lines**: parser/mod.rs:69-273; precedence.rs (full); primary.rs:239-263

## Description
`parse_query`, `parse_parenthesized_expression`, `parse_unparenthesized_subquery`, and the precedence cascade recurse without bound on user input.

## Exploit Scenario
A 100 KB query body of nested parentheses such as `((((((1))))))…` or nested `FOR`/subqueries crashes the server with stack overflow (Rust's default 8 MB thread stack triggers an abort, not a recoverable panic).

## Recommendation
Add a `depth: usize` field on `Parser`, increment in each recursive descent call, and return `ParseError("Query nesting too deep")` once the depth exceeds e.g. 64.

## References
- Related: SEC-094 (query timeout).
