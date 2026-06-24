/// Normalized Web Mercator projection helpers.
///
/// The formula here is identical to walkers' internal `mercator_normalized()`.
/// Pre-computing these coordinates at data-load time lets the per-frame
/// renderer replace the full trigonometric projection with a cheap affine
/// transform (two multiplies + two adds per point).
use crate::coordinates::{Latitude, Longitude};
use std::f64::consts::PI;

/// A pre-computed normalized Web Mercator position.
///
/// Both fields are in `[0.0, 1.0]`:
/// - `x` increases west → east (0 = 180° W, 0.5 = 0°, 1 = 180° E)
/// - `y` increases north → south (0 = top, 0.5 = equator, 1 = bottom)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercPoint {
    pub x: f64,
    pub y: f64,
}

/// Convert a geographic position to a normalized Web Mercator [`MercPoint`].
///
/// Both outputs are in `[0.0, 1.0]`:
/// - `x` increases west → east (0 = 180° W, 0.5 = 0°, 1 = 180° E)
/// - `y` increases north → south (0 = top, 0.5 = equator, 1 = bottom)
///
/// The anchor point (lon = 0°, lat = 0°) maps to exactly `(0.5, 0.5)`, which
/// is used by the renderers to turn a single `projector.project(lat_lon(0,0))`
/// call into the per-frame screen → Mercator offset.
pub fn normalize(lat: Latitude, lon: Longitude) -> MercPoint {
    let x = lon.as_degrees().to_radians();
    let y = lat.as_degrees().to_radians().tan().asinh();
    MercPoint {
        x: (1.0 + x / PI) / 2.0,
        y: (1.0 - y / PI) / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{Latitude, Longitude, normalize};

    proptest::proptest! {
        /// `normalize` must never return NaN or Inf for any geographically valid input.
        ///
        /// Web Mercator is undefined above ≈85.05°. Lat is clamped to [-85, 85]
        /// to stay within the finite range of `tan.asinh()`.
        #[test]
        fn normalize_is_finite_for_valid_inputs(
            lon in -180.0_f64..=180.0_f64,
            lat in -85.0_f64..=85.0_f64,
        ) {
            let pt = normalize(Latitude::new(lat), Longitude::new(lon));
            proptest::prop_assert!(pt.x.is_finite(), "x is not finite for lon={lon}, lat={lat}");
            proptest::prop_assert!(pt.y.is_finite(), "y is not finite for lon={lon}, lat={lat}");
        }

        /// x must lie in [0, 1]: west edge (lon = -180) → 0, east edge → 1.
        #[test]
        fn normalize_x_in_unit_range(lon in -180.0_f64..=180.0_f64) {
            let pt = normalize(Latitude::new(0.0), Longitude::new(lon));
            proptest::prop_assert!(pt.x >= 0.0 && pt.x <= 1.0, "x={} out of [0,1] for lon={lon}", pt.x);
        }

        /// y must lie in [0, 1]: north (lat = 85) → near 0, south → near 1.
        #[test]
        fn normalize_y_in_unit_range(lat in -85.0_f64..=85.0_f64) {
            let pt = normalize(Latitude::new(lat), Longitude::new(0.0));
            proptest::prop_assert!(pt.y >= 0.0 && pt.y <= 1.0, "y={} out of [0,1] for lat={lat}", pt.y);
        }
    }

    #[test]
    fn normalize_origin_maps_to_half() {
        let pt = normalize(Latitude::new(0.0), Longitude::new(0.0));
        assert!((pt.x - 0.5).abs() < 1e-12);
        assert!((pt.y - 0.5).abs() < 1e-12);
    }

    #[test]
    fn normalize_x_increases_east() {
        let pt1 = normalize(Latitude::new(0.0), Longitude::new(-90.0));
        let pt2 = normalize(Latitude::new(0.0), Longitude::new(90.0));
        assert!(pt1.x < pt2.x);
    }

    #[test]
    fn normalize_y_increases_south() {
        let pt_north = normalize(Latitude::new(60.0), Longitude::new(0.0));
        let pt_south = normalize(Latitude::new(-60.0), Longitude::new(0.0));
        assert!(pt_north.y < pt_south.y);
    }
}
