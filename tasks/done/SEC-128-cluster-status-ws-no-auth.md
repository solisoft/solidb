# SEC-128: `/_api/cluster/status/ws` exposes cluster info without auth

## Status
- **Severity**: HIGH
- **Category**: Information Disclosure / Authentication Bypass
- **Project**: soli/db
- **File**: `src/server/routes.rs`, `src/server/handlers/websocket.rs`
- **Lines**: routes.rs:1048; websocket.rs:24-74 (`cluster_status_ws`)

## Description
`cluster_status_ws` is mounted in the public router and never validates a token nor an `Origin`, unlike `monitor_ws_handler` and `ws_changefeed_handler` which both require auth. The handler streams node ID, version, peer addresses, doc counts, and disk paths every second to anyone who can connect.

## Exploit Scenario
An unauthenticated remote attacker connects to `wss://target/_api/cluster/status/ws` and obtains a continuous feed of cluster topology and resource metrics — useful for reconnaissance and DoS targeting.

## Recommendation
Mirror the pattern from `monitor_ws_handler`: require a valid JWT/API-key and call `validate_ws_origin` before upgrading.

## References
- Related: SEC-086, SEC-091.
