#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

// Feed arbitrary bytes to the `.gtd` decoder. It must return `Ok`/`Err`, never
// panic or abort. Mirrors the stable `tests/decode_robustness.rs` checks.
fuzz_target!(|data: &[u8]| {
    let _ = geotrace_sdk::NavFile::read(Cursor::new(data));
});
