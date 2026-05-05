# SEC-077: SSRF via solidb.fetch() - No URL Validation

## Status
- **Severity**: CRITICAL
- **Category**: SSRF
- **Project**: soli/db
- **File**: `src/scripting/lua_globals/http.rs`
- **Lines**: 1-65

## Description
The `solidb.fetch()` function allows Lua scripts to make HTTP requests to arbitrary URLs without any restrictions. This enables Server-Side Request Forgery (SSRF) attacks.

## Exploit Scenario
```lua
-- Access internal services
local response = solidb.fetch("http://169.254.169.254/latest/meta-data/")
-- Access localhost services
local response = solidb.fetch("http://127.0.0.1:6379/")
```

## Recommendation
Add URL validation whitelist or restrict to allowed domains/ports.

## References
- Related: SEC-007, SEC-015, SEC-016 (existing SSRF issues in lang)