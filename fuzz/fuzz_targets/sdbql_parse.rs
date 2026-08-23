#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the full SDBQL pipeline (lexer + parser) with arbitrary input.
//
// The parser must return `Err` on malformed input — it must never panic,
// hang, or abort. Recursion depth is capped (`MAX_PARSE_DEPTH`), so deeply
// nested expressions are rejected, not a stack overflow.
fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Result is intentionally discarded: we only care that parsing
        // neither panics nor aborts.
        let _ = solidb::sdbql::parser::parse(input);
    }
});
