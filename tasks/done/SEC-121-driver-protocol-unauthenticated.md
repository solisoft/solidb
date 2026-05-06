# SEC-121: Native driver protocol skips authentication

## Status
- **Severity**: CRITICAL
- **Category**: Authentication Bypass
- **Project**: soli/db
- **File**: `src/driver/handlers/mod.rs`
- **Lines**: 130-260 (`execute_command`)

## Description
The MessagePack-based binary driver protocol exposes commands such as `Insert`, `Delete`, `CreateDatabase`, `DeleteDatabase`, and `Query`. The handler tracks an `authenticated_db` field on the connection state but never checks it before dispatching commands — the `Auth` command sets the field but no other command verifies it.

Any client that speaks the magic header `solidb-drv-v1\0` on the multiplexed sync port can read and write the entire database without credentials.

## Exploit Scenario
```text
client TCP-connects to sync port
client sends: solidb-drv-v1\0
client sends: Command::Insert { db, collection, doc }   (no Auth first)
server inserts the document
```

## Recommendation
Reject every non-`Ping`/non-`Auth` command if `self.authenticated_db.is_none()`. Treat the binary driver path with the same auth posture as the HTTP API.

## References
- Related: SEC-081 (keyfile required for sync), SEC-108 (TOFC pattern).
