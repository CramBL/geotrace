//! The factors the SDKs convert speeds with.
//!
//! `examples/generate_bindings.rs` writes each factor into the C++ SDK as
//! `Velocity::kMpsPerKmh` and `Velocity::kMpsPerKnot`. `kmh * MPS_PER_KMH` and
//! `kmh / 3.6` differ in the last place for some values, 23.2 among them.

/// Meters per second in one kilometer per hour.
pub const MPS_PER_KMH: f64 = 1.0 / 3.6;

/// Meters per second in one knot, a nautical mile of 1852 m per hour.
pub const MPS_PER_KNOT: f64 = 1852.0 / 3600.0;
