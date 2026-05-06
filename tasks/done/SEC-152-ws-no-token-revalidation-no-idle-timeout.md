# SEC-152: WebSocket sessions never re-validate auth and have no idle timeout

## Status
- **Severity**: MEDIUM
- **Category**: Authentication / Resource Exhaustion
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`
- **Lines**: 254-365 (`handle_socket`); `monitor_ws_handler`

## Description
Token validation runs once at upgrade. If the JWT later expires or the API key is revoked, the connection keeps streaming. There is also no max session length and no pong-timeout (only a 30-s server ping with no liveness enforcement).

## Exploit Scenario
- A user is removed; their open WS keeps receiving live data until a process restart.
- An attacker holds many connections idle to exhaust file descriptors / memory.

## Recommendation
- Re-validate the token periodically (every 5–15 min) and close on revocation or expiry.
- Track last-pong; close the connection if no pong arrives within `2 * ping_interval`.
- Enforce a maximum session age (e.g., 24 h).

## References
- Related: SEC-117, SEC-129.
