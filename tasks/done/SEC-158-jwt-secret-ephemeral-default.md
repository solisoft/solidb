# SEC-158: JWT secret defaults to an ephemeral random value

## Status
- **Severity**: MEDIUM
- **Category**: Authentication / Configuration
- **Project**: soli/db
- **File**: `src/server/auth.rs`
- **Lines**: 115-143 (`JWT_SECRET` initialization)

## Description
When `JWT_SECRET` is not set, the server generates a fresh random value at startup. This silently invalidates all tokens on every restart (operator surprise) and, more importantly, fails open in production deployments that forget the env var: the server keeps serving with an unaudited secret while only emitting a `tracing::warn!`.

## Recommendation
- Refuse to start in production mode unless `JWT_SECRET` is configured. Detect production by either an explicit `SOLIDB_ENV=production` or by the presence of a release-build conditional.
- In dev mode, persist the generated secret to `data_dir/.jwt_secret` (mode 0600) so restarts don't break clients.

## References
- Related: SEC-106.
