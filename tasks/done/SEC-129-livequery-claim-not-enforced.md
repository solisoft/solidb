# SEC-129: `livequery` claim is never enforced

## Status
- **Severity**: HIGH
- **Category**: Authorization
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 150 (claim definition), 651 (token issuance), 665-674 (`validate_token`)

## Description
The `/_api/livequery/token` endpoint mints JWTs with `livequery: Some(true)` and a 2-second TTL. `validate_token` returns the claims unchanged and downstream handlers never inspect `claims.livequery`. The 2-second TTL is a soft mitigation rather than a security boundary — within that window the token is accepted as a fully privileged JWT (and gains admin via SEC-124).

## Exploit Scenario
A user requests a livequery token, then within 2 seconds calls `DELETE /_api/database/{db}` using that same token — request is accepted.

## Recommendation
In `auth_middleware`, when `claims.livequery == Some(true)`, allow the request only when the path is on a strict whitelist (currently `/_api/ws/changefeed` and possibly `/_api/livequery/*`). Reject everywhere else with 403.

## References
- Related: SEC-117, SEC-124.
