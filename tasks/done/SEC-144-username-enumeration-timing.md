# SEC-144: Username enumeration via login-handler timing

## Status
- **Severity**: MEDIUM
- **Category**: Information Disclosure
- **Project**: soli/db
- **File**: `src/server/handlers/auth.rs`
- **Lines**: 329-348

## Description
On unknown user, `login_handler` returns "Invalid credentials" without running Argon2 verification. On a valid username with the wrong password, Argon2 runs (~50–200 ms). The timing difference reliably reveals which usernames exist.

## Exploit Scenario
An attacker scripts logins with a wordlist of usernames and any password, measuring response times. Usernames with high-latency responses are valid accounts → focused brute-forcing.

## Recommendation
When the user lookup fails, run a dummy `verify_password(&req.password, DUMMY_HASH)` against a precomputed Argon2 hash to equalize timing. Use a constant dummy hash baked at startup.

## References
- Related: SEC-093.
