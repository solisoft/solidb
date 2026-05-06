# SEC-159: Argon2 password hashes use library-default parameters

## Status
- **Severity**: MEDIUM
- **Category**: Cryptographic
- **Project**: soli/db
- **File**: `src/server/auth.rs`, `src/scripting/lua_globals/crypto.rs`
- **Lines**: auth.rs:279, 590, 598; crypto.rs:224, 246

## Description
Calls use `Argon2::default()`, which selects the argon2 crate's defaults (m=19 MiB, t=2, p=1). OWASP's 2024 password-hash guidance treats these as a floor, recommending higher memory/time costs for high-value accounts. For an admin password store this is borderline; raising parameters now buys a year+ of margin against GPU/ASIC progress.

## Recommendation
Use explicit `Params::new(65536, 3, 4, None).unwrap()` (m=64 MiB, t=3, p=4) at hash sites that handle `_admins`. Keep the `Argon2::default()` for low-value Lua use if needed.

## References
- Related: SEC-093, SEC-106.
