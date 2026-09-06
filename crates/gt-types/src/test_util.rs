#![cfg(test)]
//! Assertions shared between the test modules of gt-types.

/// 1e-9° is about 0.1 mm.
pub const DEGREES_TOLERANCE: f64 = 1e-9;

pub fn assert_degrees_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < DEGREES_TOLERANCE,
        "expected {expected}°, got {actual}°"
    );
}
