/// Normalized Web Mercator projection helpers.
///
/// The formula here is identical to walkers' internal `mercator_normalized()`.
/// Pre-computing these coordinates at data-load time lets the per-frame
/// renderer replace the full trigonometric projection with a cheap affine
/// transform (two multiplies + two adds per point).
use std::f64::consts::PI;

/// Convert a geographic position to normalized Web Mercator coordinates.
///
/// Both outputs are in `[0.0, 1.0]`:
/// - `x` increases west → east (0 = 180° W, 0.5 = 0°, 1 = 180° E)
/// - `y` increases north → south (0 = top, 0.5 = equator, 1 = bottom)
///
/// The anchor point (lon = 0°, lat = 0°) maps to exactly `(0.5, 0.5)`, which
/// is used by the renderers to turn a single `projector.project(lat_lon(0,0))`
/// call into the per-frame screen → Mercator offset.
pub(crate) fn normalize(lon_deg: f64, lat_deg: f64) -> (f64, f64) {
    let x = lon_deg.to_radians();
    let y = lat_deg.to_radians().tan().asinh();
    let x = (1.0 + x / PI) / 2.0;
    let y = (1.0 - y / PI) / 2.0;
    (x, y)
}
