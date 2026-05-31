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

#[cfg(test)]
mod tests {
    use super::normalize;

    proptest::proptest! {
        /// `normalize` must never return NaN or Inf for any geographically valid input.
        ///
        /// Web Mercator is undefined above ≈85.05°; lat is clamped to [-85, 85]
        /// to stay within the finite range of `tan.asinh()`.
        #[test]
        fn normalize_is_finite_for_valid_inputs(
            lon in -180.0_f64..=180.0_f64,
            lat in -85.0_f64..=85.0_f64,
        ) {
            let (x, y) = normalize(lon, lat);
            proptest::prop_assert!(x.is_finite(), "x is not finite for lon={lon}, lat={lat}");
            proptest::prop_assert!(y.is_finite(), "y is not finite for lon={lon}, lat={lat}");
        }

        /// x must lie in [0, 1]: west edge (lon = -180) → 0, east edge → 1.
        #[test]
        fn normalize_x_in_unit_range(lon in -180.0_f64..=180.0_f64) {
            let (x, _) = normalize(lon, 0.0);
            proptest::prop_assert!(x >= 0.0 && x <= 1.0, "x={x} out of [0,1] for lon={lon}");
        }

        /// y must lie in [0, 1]: north (lat = 85) → near 0, south → near 1.
        #[test]
        fn normalize_y_in_unit_range(lat in -85.0_f64..=85.0_f64) {
            let (_, y) = normalize(0.0, lat);
            proptest::prop_assert!(y >= 0.0 && y <= 1.0, "y={y} out of [0,1] for lat={lat}");
        }
    }

    #[test]
    fn normalize_origin_maps_to_half() {
        let (x, y) = normalize(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-12);
        assert!((y - 0.5).abs() < 1e-12);
    }

    #[test]
    fn normalize_x_increases_east() {
        let (x1, _) = normalize(-90.0, 0.0);
        let (x2, _) = normalize(90.0, 0.0);
        assert!(x1 < x2);
    }

    #[test]
    fn normalize_y_increases_south() {
        let (_, y_north) = normalize(0.0, 60.0);
        let (_, y_south) = normalize(0.0, -60.0);
        assert!(y_north < y_south);
    }
}
