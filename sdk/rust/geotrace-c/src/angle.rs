//! Angle conversions between radians and degrees. A `.gtd` file stores an angle in degrees.

use geotrace_sdk::Angle;

/// Convert an angle in radians to degrees.
///
/// The Rust SDK's `Angle::radians` computes the same double.
///
/// @param radians Angle in radians.
///
/// @return The angle in degrees.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_degrees_from_radians(radians: f64) -> f64 {
    Angle::radians(radians).as_degrees()
}

/// Convert an angle in degrees to radians.
///
/// The Rust SDK's `Angle::as_radians` computes the same double.
///
/// @param degrees Angle in degrees.
///
/// @return The angle in radians.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_radians_from_degrees(degrees: f64) -> f64 {
    Angle::degrees(degrees).as_radians()
}
