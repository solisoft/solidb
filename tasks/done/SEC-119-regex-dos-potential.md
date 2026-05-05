# SEC-119: Regex DoS Potential

## Status
- **Severity**: MEDIUM
- **Category**: DoS
- **Project**: soli/db
- **Files**: `src/sdbql/executor/utils.rs`, `src/sdbql/executor/helpers.rs`
- **Lines**: 16-29, 135-151

## Description
While `safe_regex` limits pattern length (1024 bytes) and compiled size (1MB), certain regex patterns like `(a+)+$` can cause time complexity attacks.

## Exploit Scenario
Carefully crafted regex causes exponential backtracking.

## Recommendation
Add complexity limits beyond size limits, consider using `heuristics` feature of regex crate.

## References
- Related: SEC-105 (no rate limit query parsing)