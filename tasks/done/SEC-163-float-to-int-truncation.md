# SEC-163: Float-to-int truncation in array index / range expressions

## Status
- **Severity**: LOW
- **Category**: Logic / Defensive
- **Project**: soli/db
- **File**: `src/sdbql/executor/expression.rs`
- **Lines**: 149-163, 273-302

## Description
`f64 as usize` / `f64 as i64` is saturating in modern Rust (no UB), but yields silently wrong indices for `NaN` (→ 0) and out-of-range floats. This is unlikely to be directly exploitable but produces hard-to-debug query results.

## Recommendation
When converting a float to an index, reject non-finite values explicitly and return a query error.

## References
- Related: SEC-118.
