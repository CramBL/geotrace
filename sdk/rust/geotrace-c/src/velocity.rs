//! Speed conversions to and from m/s, the unit a `.gtd` file stores a speed in.

use geotrace_sdk::Velocity;

/// Convert a speed in km/h to m/s.
///
/// Every SDK converts @p kmh to the same double. `kmh / 3.6` differs from it
/// in the last place for some values, 23.2 among them.
///
/// @param kmh Speed in km/h.
///
/// @return The speed in m/s.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_mps_from_kmh(kmh: f64) -> f64 {
    Velocity::kilometer_per_hour(kmh).as_meters_per_second()
}

/// Convert a speed in knots to m/s.
///
/// Every SDK converts @p knots to the same double.
///
/// @param knots Speed in knots, nautical miles of 1852 m per hour.
///
/// @return The speed in m/s.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_mps_from_knots(knots: f64) -> f64 {
    Velocity::knot(knots).as_meters_per_second()
}

/// Convert a speed in m/s to km/h.
///
/// @param mps Speed in m/s.
///
/// @return The speed in km/h.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_kmh_from_mps(mps: f64) -> f64 {
    Velocity::meter_per_second(mps).as_kilometers_per_hour()
}

/// Convert a speed in m/s to knots.
///
/// @param mps Speed in m/s.
///
/// @return The speed in knots.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_knots_from_mps(mps: f64) -> f64 {
    Velocity::meter_per_second(mps).as_knots()
}
