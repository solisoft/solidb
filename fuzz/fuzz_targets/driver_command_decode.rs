#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the binary protocol command decoder.
//
// The driver decodes attacker-controllable bytes from the TCP port into
// `Command` values. Any input must decode to `Ok` or `Err` — never panic.
// Frame-length caps live in the codec; here we exercise the MessagePack
// layer directly, including deeply nested payloads that `serde` recursion
// limits should reject as errors.
fuzz_target!(|data: &[u8]| {
    let _: Result<solidb::driver::protocol::Command, _> =
        solidb::driver::protocol::decode_message(data);
});
