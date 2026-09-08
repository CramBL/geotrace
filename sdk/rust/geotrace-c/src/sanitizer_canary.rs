//! The leaking entry point of the sanitizer canary, behind the
//! `sanitizer_canary` feature.

use std::hint;

const LEAKED_BYTES: usize = 64;

/// Leaks one heap allocation, so that the address sanitizer prints a leak
/// report whose allocation stack reaches into this library.
///
/// `leak_canary.c` of the C test suite calls this, and the canary fails when
/// the report contains no Rust source file.
///
/// `cbindgen` reads this module whatever features are enabled, and the
/// annotation below keeps the entry point out of `geotrace.h`.
///
/// cbindgen:ignore
#[unsafe(no_mangle)]
pub extern "C" fn gtd_leak_allocation_for_sanitizer_canary() {
    hint::black_box(Box::leak(Box::new([0_u8; LEAKED_BYTES])));
}
