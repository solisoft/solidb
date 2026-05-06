# SEC-127: REPL endpoint accepts any authenticated user

## Status
- **Severity**: HIGH
- **Category**: Authorization
- **Project**: soli/db
- **File**: `src/server/script_handlers.rs`
- **Lines**: 468-471 (`repl_eval_handler`)

## Description
`/_api/database/{db}/repl` runs arbitrary Lua against `state.storage` and ignores `claims` (`_claims` parameter). Any token holder — including a viewer-role API key or short-lived `livequery` token — can execute arbitrary Lua, effectively bypassing all RBAC.

## Exploit Scenario
Viewer API key holder POSTs `solidb.delete_database("production")` to the REPL.

## Recommendation
- Require admin role (or at minimum `Write` on the target database).
- Explicitly reject claims with `livequery == Some(true)`.
- Consider gating the REPL behind a feature flag in production builds.

## References
- Related: SEC-120, SEC-124, SEC-126, SEC-129.
