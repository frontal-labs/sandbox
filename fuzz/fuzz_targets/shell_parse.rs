#![no_main]

use libfuzzer_sys::fuzz_target;

// The parser must return an error for any input, never panic or loop.
fuzz_target!(|data: &[u8]| {
    if let Ok(script) = std::str::from_utf8(data) {
        let _ = sandbox_shell::parse(script);
    }
});
