# Login rate limit: one 127.0.0.1 bucket starves all local processes

## Severity

low (after client-side fixes) — was the trigger of a self-sustaining `/auth/login` 400 storm

## Location

- `src/server/auth.rs` — `check_rate_limit` (`MAX_LOGIN_ATTEMPTS = 20`, `RATE_LIMIT_WINDOW_SECS = 60`)
- `src/server/handlers/auth.rs` — `login_handler`

## Problem

The limiter keys on client IP. Local dev/test traffic all comes from
127.0.0.1, so `soli test --jobs N` (1 runner + N server processes, each
needing a JWT) shares one 20/min bucket. Once a client's login fails it
used to retry per query (fixed soli-lang side: failure backoff +
runner-minted `SOLIDB_JWT` shared with children), keeping every local
process in permanent 400.

Also: the 400 body ("Too many login attempts...") is indistinguishable
from "Invalid credentials" at the status-code level — clients can't tell
"back off" from "wrong password". 429 with `Retry-After` would be the
honest signal.

## Possible directions

- Return 429 (+ `Retry-After`) for rate limiting instead of 400.
- Key the bucket on (ip, username) or exempt successful logins from the
  count so N legitimate parallel logins don't lock the door.
- Make `MAX_LOGIN_ATTEMPTS` / window configurable via env.
