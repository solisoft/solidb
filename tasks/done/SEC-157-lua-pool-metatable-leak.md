# SEC-157: Lua pool reset preserves user-set metatables across tenants

## Status
- **Severity**: MEDIUM
- **Category**: Multi-tenant Isolation
- **Project**: soli/db
- **File**: `src/scripting/engine/pool.rs`
- **Lines**: 566-642 (`reset_state`)

## Description
`reset_state` clears non-preserved keys from `globals()` but does not restore metatables on built-in primitive types. A previous request can do:
```lua
getmetatable("").__index = function(s, k)
  return rawget(string, k) or some_logger(s)
end
```
The next request reusing the same pool slot inherits this injected metatable — its `s:sub(...)` calls leak data to the previous tenant's logger.

## Exploit Scenario
Tenant A monkey-patches `string` metatable to exfiltrate strings; tenant B's script (same pool slot) calls a string method and the contents flow to A's webhook.

## Recommendation
- On reset, call `setmetatable(getmetatable(""), nil)` for each base type.
- Or recreate the Lua state when metatable mutation is detected.
- Best: switch to ephemeral states for cross-tenant requests.

## References
- Related: SEC-156.
