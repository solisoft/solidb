# COV-004: Cover `server/handlers/websocket.rs` (0% → ≥40%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`
- **Current coverage**: 0% (632 lines uncovered)

## Description
WebSocket changefeed and live-query endpoints have no automated tests. SEC-152 added re-validation and idle-timeout to WS connections; both pieces and the query-token gating live in this file.

## Recommendation
Add `tests/websocket_handlers_tests.rs` driven by `tokio-tungstenite` against an axum server bound to an ephemeral port (see how integration suites that need real sockets work today — search for `tokio::net::TcpListener` in `tests/`). Where the harness is too heavy, extract pure helpers from `websocket.rs` (token-gate predicate, idle-timeout calculator, message validator) and unit-test those.

Cases to exercise:
- WS connect with valid livequery JWT → upgrade succeeds; with missing/expired token → 401/403 (SEC-152).
- Idle-timeout disconnects after N seconds of silence (use a short timeout in tests).
- Re-validation: after the JWT expires mid-connection, the next message is rejected and the socket closes.
- Subscribe to a changefeed; insert via HTTP; assert WS receives the event.
- Malformed inbound frame → graceful error.

## Goal
Raise `src/server/handlers/websocket.rs` to ≥40% line coverage. (Pure-helper unit tests can push this further without spinning up a full WS harness.)

## References
- Related: SEC-152, SEC-153
- Pattern: existing async + axum tests in `tests/`
