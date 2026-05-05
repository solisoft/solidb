# SEC-114: Ollama URL Scheme Prepends HTTP Without Validation

## Status
- **Severity**: MEDIUM
- **Category**: SSRF
- **Project**: soli/db
- **File**: `src/server/llm_client.rs`
- **Lines**: 174-180

## Description
If a user configures `OLLAMA_URL` with `localhost:11434/internal/evil`, it prepends `http://` making `http://localhost:11434/internal/evil`.

## Exploit Scenario
Redirection to malicious internal endpoints.

## Recommendation
Validate URLs against blocklist of internal addresses.

## References
- Related: SEC-077 (ssrf solidb fetch), SEC-102 (blob chunk ssrf)