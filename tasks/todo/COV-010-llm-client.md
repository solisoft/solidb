# COV-010: Cover `server/llm_client.rs` (0% → ≥60%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/llm_client.rs`
- **Current coverage**: 0% (353 lines uncovered)

## Description
The LLM client used by NL query handlers (COV-007) and any AI features has no test coverage. It does HTTP I/O, retries, and parses provider responses — all easily wrong, all silently.

## Recommendation
Add `tests/llm_client_tests.rs` driven by `wiremock` (or a minimal axum stub) bound to an ephemeral port. Construct the client pointing at that URL and assert:

- Successful completion call: stub returns canned JSON, client parses and returns the expected struct.
- Network error → returns error variant (no panic).
- 5xx response → retries up to the configured limit, then fails.
- 4xx response → fails immediately, no retry.
- Malformed JSON in 200 response → parse error surfaces as a typed error.
- API key / auth header is set on outgoing requests (assert via `wiremock` matcher).
- Request timeout fires (use a slow stub).

If the client doesn't currently take an injectable base URL or `reqwest::Client`, add that knob — it's both easier to test and useful in production for proxying.

## Goal
Raise `src/server/llm_client.rs` to ≥60% line coverage.

## References
- Companion: COV-007 (`server/nl_handlers.rs`)
