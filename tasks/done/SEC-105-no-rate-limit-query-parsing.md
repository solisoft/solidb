# SEC-105: No Rate Limiting on Query Parsing

## Status
- **Severity**: MEDIUM
- **Category**: DoS
- **Project**: soli/db
- **File**: `src/sdbql/parser/mod.rs`
- **Lines**: 24-33, 37-48

## Description
The prepared statement cache doesn't rate-limit parsing requests. An attacker could exhaust CPU with rapid parse requests.

## Exploit Scenario
Rapid malformed query submissions cause CPU exhaustion.

## Recommendation
Add rate limiting on query parsing.

## References
- Related: SEC-092 (no rate limit admin endpoints), SEC-030 (rate limit xff spoof)