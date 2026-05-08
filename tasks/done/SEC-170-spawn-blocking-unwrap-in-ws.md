# SEC-170: `spawn_blocking` join unwrap in WebSocket request path

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`
- **Lines**: 899

## Description
A `tokio::task::spawn_blocking(...).await.unwrap()` runs in the WebSocket request path. A panic inside the blocking worker becomes a `JoinError`, which the unwrap converts into a panic of the request task — taking the WS connection (or worse, the whole task) down.

## Recommendation
Propagate the `JoinError` via `?` and convert to a `DbError::InternalError(...)` returning 500 to the client.

## References
- Related: SEC-094.
