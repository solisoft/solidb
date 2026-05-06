# SEC-156: Lua sandbox has no memory limit and no interrupt

## Status
- **Severity**: MEDIUM
- **Category**: Denial of Service
- **Project**: soli/db
- **File**: `src/scripting/engine/repl.rs`, `src/scripting/engine/websocket.rs`, `src/scripting/engine/mod.rs`
- **Lines**: repl.rs:36; websocket.rs:66; mod.rs:225

## Description
All entry points construct the Lua state via `Lua::new()` (full stdlib), then nil out specific globals (`os, io, debug, package, dofile, load, loadfile, require`). This is a substantive defense-in-depth gap because:
- Future mlua additions will silently land as accessible globals.
- `string.dump` remains usable.
- `collectgarbage("setpause", 0)` / `("setstepmul", huge)` is still callable and lets a script trash GC.
- There is **no `lua.set_memory_limit`** and **no `lua.set_interrupt`** for CPU bounds.

## Exploit Scenario
- `local s = string.rep("a", 2^28)` allocates 256 MB per request; pool reset retains the state, compounding across requests.
- `while true do end` in a sync-evaluation path hangs a worker thread forever.

## Recommendation
- Replace `Lua::new()` with `Lua::new_with(StdLib::MATH | STRING | TABLE | OS_DATETIME | ...)` containing only required libraries.
- Call `lua.set_memory_limit(64 * 1024 * 1024)` per state.
- Call `lua.set_interrupt(...)` with a deadline derived from per-script CPU budget.
- Restrict `collectgarbage` arguments via a wrapper.

## References
- Related: SEC-120.
