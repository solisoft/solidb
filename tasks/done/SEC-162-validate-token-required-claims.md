# SEC-162: `validate_token` does not pin required spec claims

## Status
- **Severity**: LOW
- **Category**: Cryptographic / Defense in Depth
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 665-674

## Description
`Validation::new(Algorithm::HS256)` is used as-is. The current `jsonwebtoken` default sets `validate_exp = true`, but the validation spec is otherwise loose: tokens with `exp == usize::MAX` (used internally for API keys / cluster claims) are accepted forever via the same path that serves user JWTs.

## Recommendation
Pin `validation.required_spec_claims = HashSet::from(["exp", "sub"])` and reject `exp == usize::MAX` for `Bearer` JWTs. Internal cluster/API claims should never flow through the user JWT validation path.

## References
- Related: SEC-106, SEC-129.
