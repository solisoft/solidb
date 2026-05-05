# SEC-095: Open Redirect via solidb.redirect()

## Status
- **Severity**: MEDIUM
- **Category**: Open Redirect
- **Project**: soli/db
- **File**: `src/scripting/http_helpers.rs`
- **Lines**: 60-65

## Description
The redirect function accepts arbitrary URLs without validation, enabling phishing attacks via open redirect.

## Exploit Scenario
```lua
solidb.redirect("https://evil-attacker.com/phishing-page")
```

## Recommendation
Validate redirect URLs against an allowlist of permitted domains.

## References
- Related: SEC-012 (link to javascript xss)