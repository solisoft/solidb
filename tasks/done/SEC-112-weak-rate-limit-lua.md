# SEC-112: Weak Rate Limiting in Lua Scripting

## Status
- **Severity**: MEDIUM
- **Category**: Rate Limiting
- **Project**: soli/db
- **File**: `src/scripting/error_handling.rs`
- **Lines**: 119-175

## Description
The rate limiter uses a static global `OnceLock` without per-database or per-user isolation. Attackers can bypass by using different identifiers.

## Exploit Scenario
```lua
for i = 1, 1000 do
    solidb.rate_limit("user" .. math.random(), 100, 60)
end
```

## Recommendation
Implement per-user/per-IP rate limiting with proper isolation.

## References
- Related: SEC-092 (no rate limit admin endpoints), SEC-105 (no rate limit query parsing)