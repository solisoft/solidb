# SEC-087: No WebSocket Message Size Limits

## Status
- **Severity**: HIGH
- **Category**: DoS
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`
- **Lines**: 237-281

## Description
Messages are processed without any size validation, enabling OOM attacks via massive messages.

## Exploit Scenario
An attacker sends massive messages to exhaust server memory.

## Recommendation
Add message size limits using `Message::max_size()` or similar.

## References
- Related: SEC-047 (ws no max message size) in lang