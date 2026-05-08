# COV-007: Cover `server/nl_handlers.rs` (0% → ≥50%)

## Status
- **Category**: Test Coverage
- **Project**: soli/db
- **File**: `src/server/nl_handlers.rs`
- **Current coverage**: 0% (298 lines uncovered)

## Description
Natural-language query handlers (NL → SDBQL translation, schema introspection) have no test coverage. The LLM client this depends on (`src/server/llm_client.rs`) is also at 0% — see COV-010.

## Recommendation
Add `tests/nl_handlers_tests.rs`. Because NL handlers call out to an LLM, prefer one of:
1. Inject a fake LLM client (introduce a trait + test impl) and assert the handler glue (request validation, schema collection, prompt assembly, response parsing).
2. Use a stubbed HTTP server (e.g. `wiremock`) bound to an ephemeral port and point `llm_client` at it via configuration.

Cases to exercise:
- Valid NL query → handler builds the expected prompt (assert via fake) → returns SDBQL.
- Schema collection for a non-existent DB → 404.
- LLM returns malformed JSON → handler responds 502/400 instead of panicking.
- AuthZ: missing JWT → 401.
- Rate-limit / size-cap on the NL prompt (if implemented).

## Goal
Raise `src/server/nl_handlers.rs` to ≥50% line coverage.

## References
- Companion: COV-010 (`server/llm_client.rs`)
- Pattern: `tests/handlers_tests.rs`
