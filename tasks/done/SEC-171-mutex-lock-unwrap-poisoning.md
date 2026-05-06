# SEC-171: `state.system_monitor.lock().unwrap()` panics propagate via mutex poisoning

## Status
- **Severity**: LOW
- **Category**: Reliability
- **Project**: soli/db
- **File**: `src/server/handlers/websocket.rs`, `src/server/handlers/cluster.rs`, `src/server/handlers/sharding.rs`, `src/server/metrics.rs` (~10 sites)

## Description
The system monitor (and a few other shared resources) use `std::sync::Mutex` and call `.lock().unwrap()` everywhere. A panic while holding the lock poisons it permanently; subsequent requests panic on every lock attempt.

## Recommendation
- Use `parking_lot::Mutex` (no poisoning).
- Or handle `PoisonError` by recovering the inner value (`.unwrap_or_else(|e| e.into_inner())`).

## References
- Related: SEC-094.
