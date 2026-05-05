# SEC-086: No WebSocket Origin Validation

## Status
- **Severity**: HIGH
- **Category**: WebSocket Security
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`
- **Lines**: 20-26, 75-88, 162-199

## Description
WebSocket handlers (`cluster_status_ws`, `monitor_ws_handler`, `ws_changefeed_handler`) do not validate the `Origin` header.

## Exploit Scenario
A malicious website could establish WebSocket connections to access real-time changefeeds.

## Recommendation
Add origin validation and use `validate_origin` function pattern.

## References
- Related: SEC-032 (ws origin trusts xfh), SEC-044 (multivalue xfp xfh)