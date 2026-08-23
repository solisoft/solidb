#![no_main]

use libfuzzer_sys::fuzz_target;
use solidb::sdbql::lexer::Lexer;

// Fuzz the SDBQL lexer in isolation.
//
// Tokenization of adversarial byte strings (unterminated strings/identifiers,
// huge numbers, control characters) must always terminate with `Ok` tokens or
// an `Err`, never a panic.
fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = Lexer::new(input).tokenize();
    }
});
