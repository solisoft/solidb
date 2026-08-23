#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the JSONL document parsing used by `solidb-restore`.
//
// Each line of a logical dump is a JSON document; restore feeds every line
// through `serde_json`. Malformed or pathological lines (deep nesting,
// huge numbers, invalid UTF-16 escapes) must produce an error, not a panic.
fuzz_target!(|data: &[u8]| {
    if let Ok(line) = std::str::from_utf8(data) {
        for chunk in line.split('\n') {
            if chunk.trim().is_empty() {
                continue;
            }
            // Same shape restore uses for dump documents.
            let _ = serde_json::from_str::<serde_json::Value>(chunk);
        }
    }
});
